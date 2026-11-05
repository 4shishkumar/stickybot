"""API keys in the macOS Keychain (a generic password, service "com.stickkey.agent").

Talks to the Security framework directly (ctypes: SecItemAdd / SecItemUpdate / SecItemCopyMatching /
SecItemDelete), never the `security` command: a key on a command line shows in the process list.

The key is write-only from the outside: the dashboard can save or remove it and see "saved (…abcd)",
but no API returns it, nothing logs it and it never goes into config.json. For development,
STICK_AGENT_OPENROUTER_KEY overrides the saved key (env_var()).

The backend is pluggable (set_backend()): tests use MemoryBackend and never reach the real Keychain.
"""
import ctypes
import logging
import os
import threading

log = logging.getLogger("keychain")

SERVICE = "com.stickkey.agent"
ENV = {"openrouter": "STICK_AGENT_OPENROUTER_KEY"}


class KeychainError(Exception):
    pass


class MemoryBackend:
    """Keys in a dict (tests)."""

    def __init__(self):
        self.items = {}

    def get(self, account):
        return self.items.get(account)

    def set(self, account, secret):
        self.items[account] = secret

    def delete(self, account):
        return self.items.pop(account, None) is not None


class MacKeychain:
    """The login Keychain through the Security framework."""

    NOT_FOUND = -25300   # errSecItemNotFound
    UTF8 = 0x08000100    # kCFStringEncodingUTF8

    def __init__(self):
        self._lock = threading.Lock()
        self._lib = None

    def _load(self):
        if self._lib:
            return self._lib
        cf = ctypes.cdll.LoadLibrary("/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation")
        sec = ctypes.cdll.LoadLibrary("/System/Library/Frameworks/Security.framework/Security")
        vp = ctypes.c_void_p
        cf.CFStringCreateWithCString.restype = vp
        cf.CFStringCreateWithCString.argtypes = [vp, ctypes.c_char_p, ctypes.c_uint32]
        cf.CFDataCreate.restype = vp
        cf.CFDataCreate.argtypes = [vp, ctypes.c_char_p, ctypes.c_long]
        cf.CFDataGetLength.restype = ctypes.c_long
        cf.CFDataGetLength.argtypes = [vp]
        cf.CFDataGetBytePtr.restype = vp
        cf.CFDataGetBytePtr.argtypes = [vp]
        cf.CFDictionaryCreate.restype = vp
        cf.CFDictionaryCreate.argtypes = [vp, ctypes.POINTER(vp), ctypes.POINTER(vp), ctypes.c_long, vp, vp]
        cf.CFRelease.restype = None
        cf.CFRelease.argtypes = [vp]
        for name in ("SecItemAdd", "SecItemCopyMatching"):
            getattr(sec, name).restype = ctypes.c_int32
            getattr(sec, name).argtypes = [vp, ctypes.POINTER(vp)]
        sec.SecItemUpdate.restype = ctypes.c_int32
        sec.SecItemUpdate.argtypes = [vp, vp]
        sec.SecItemDelete.restype = ctypes.c_int32
        sec.SecItemDelete.argtypes = [vp]
        const = lambda lib, name: vp.in_dll(lib, name).value  # the CFTypeRef the symbol holds
        self.k = {n: const(sec, n) for n in (
            "kSecClass", "kSecClassGenericPassword", "kSecAttrService", "kSecAttrAccount", "kSecAttrLabel",
            "kSecValueData", "kSecReturnData", "kSecMatchLimit", "kSecMatchLimitOne")}
        self.k["true"] = const(cf, "kCFBooleanTrue")
        # the callback structs themselves are passed by address
        self.key_cb = ctypes.addressof(vp.in_dll(cf, "kCFTypeDictionaryKeyCallBacks"))
        self.val_cb = ctypes.addressof(vp.in_dll(cf, "kCFTypeDictionaryValueCallBacks"))
        self.cf, self.sec = cf, sec
        self._lib = True
        return self._lib

    def _str(self, s, owned):
        ref = self.cf.CFStringCreateWithCString(None, s.encode(), self.UTF8)
        owned.append(ref)
        return ref

    def _data(self, b, owned):
        ref = self.cf.CFDataCreate(None, b, len(b))
        owned.append(ref)
        return ref

    def _dict(self, pairs, owned):
        n = len(pairs)
        keys = (ctypes.c_void_p * n)(*[k for k, _ in pairs])
        vals = (ctypes.c_void_p * n)(*[v for _, v in pairs])
        ref = self.cf.CFDictionaryCreate(None, keys, vals, n, self.key_cb, self.val_cb)
        owned.append(ref)
        return ref

    def _query(self, account, owned):
        k = self.k
        return [(k["kSecClass"], k["kSecClassGenericPassword"]), (k["kSecAttrService"], self._str(SERVICE, owned)),
                (k["kSecAttrAccount"], self._str(account, owned))]

    def _release(self, owned):
        for ref in owned:
            if ref:
                self.cf.CFRelease(ref)

    def get(self, account):
        with self._lock:
            self._load()
            owned = []
            try:
                q = self._dict(self._query(account, owned) + [(self.k["kSecReturnData"], self.k["true"]),
                               (self.k["kSecMatchLimit"], self.k["kSecMatchLimitOne"])], owned)
                out = ctypes.c_void_p()
                st = self.sec.SecItemCopyMatching(q, ctypes.byref(out))
                if st == self.NOT_FOUND:
                    return None
                if st != 0:
                    raise KeychainError(f"Keychain read failed (OSStatus {st})")
                owned.append(out.value)
                n = self.cf.CFDataGetLength(out.value)
                return ctypes.string_at(self.cf.CFDataGetBytePtr(out.value), n).decode()
            finally:
                self._release(owned)

    def set(self, account, secret):
        with self._lock:
            self._load()
            owned = []
            try:
                base = self._query(account, owned)
                data = self._data(secret.encode(), owned)
                st = self.sec.SecItemUpdate(self._dict(base, owned),
                                            self._dict([(self.k["kSecValueData"], data)], owned))
                if st == self.NOT_FOUND:
                    label = self._str(f"stick-key {account} API key", owned)
                    st = self.sec.SecItemAdd(self._dict(base + [(self.k["kSecValueData"], data),
                                                                (self.k["kSecAttrLabel"], label)], owned), None)
                if st != 0:
                    raise KeychainError(f"Keychain write failed (OSStatus {st})")
            finally:
                self._release(owned)

    def delete(self, account):
        with self._lock:
            self._load()
            owned = []
            try:
                st = self.sec.SecItemDelete(self._dict(self._query(account, owned), owned))
                if st not in (0, self.NOT_FOUND):
                    raise KeychainError(f"Keychain delete failed (OSStatus {st})")
                return st == 0
            finally:
                self._release(owned)


_backend = None
_cache = {}  # account -> key (read once: a Keychain read per voice request isn't needed)


def set_backend(backend):
    global _backend
    _backend = backend
    _cache.clear()


def backend():
    global _backend
    if _backend is None:
        _backend = MacKeychain()
    return _backend


def env_var(account):
    return (os.environ.get(ENV.get(account, "")) or "").strip() or None


def get(account):
    """The key (env override first), or None. Blocking: call from a worker thread in the agent."""
    key = env_var(account)
    if key:
        return key
    if account not in _cache:
        try:
            _cache[account] = backend().get(account)
        except (KeychainError, OSError, ValueError) as e:
            log.warning("%s key: %s", account, e)
            return None
    return _cache[account]


def save(account, secret):
    backend().set(account, secret)
    _cache[account] = secret
    log.info("%s key saved in the Keychain", account)


def remove(account):
    _cache.pop(account, None)
    had = backend().delete(account)
    _cache[account] = None
    log.info("%s key %s", account, "removed from the Keychain" if had else "wasn't in the Keychain")
    return had


def _describe(key, source, account):
    hint = (f"…{key[-4:]}" if len(key) >= 12 else "…") if key else None
    return {"saved": bool(key), "source": source if key else None, "hint": hint, "env_var": ENV.get(account)}


def status(account):
    """{"saved", "source": "env" | "keychain" | None, "hint": "…abcd", "env_var"} — never the key itself.
    Blocking (may read the Keychain): call from a worker thread."""
    env = env_var(account)
    return _describe(env, "env", account) if env else _describe(get(account), "keychain", account)


def known_status(account):
    """status() without reading the Keychain (for /api/status): saved None = not looked yet."""
    env = env_var(account)
    if env:
        return _describe(env, "env", account)
    if account in _cache:
        return _describe(_cache[account], "keychain", account)
    return {"saved": None, "source": None, "hint": None, "env_var": ENV.get(account)}

#include "ir_names.h"
#include <stdio.h>
#include <string.h>

namespace irnames {

static char lower(char c) { return c >= 'A' && c <= 'Z' ? c + 32 : c; }
static bool alnum(char c) { return (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9'); }

bool deviceId(const char* in, char out[DEVICE_LEN + 1]) {
  size_t k = 0;
  bool dash = false;
  for (const char* p = in ? in : ""; *p; p++) {
    char c = lower(*p);
    if (c == ' ' || c == '_' || c == '-') {
      dash = k > 0;  // no leading dash; runs collapse
      continue;
    }
    if (!alnum(c)) continue;
    if (dash) {
      if (k >= DEVICE_LEN) break;
      out[k++] = '-';
      dash = false;
    }
    if (k >= DEVICE_LEN) break;
    out[k++] = c;
  }
  while (k && out[k - 1] == '-') k--;
  out[k] = 0;
  return k > 0;
}

bool buttonName(const char* in, char out[BUTTON_LEN + 1]) {
  size_t k = 0;
  bool space = false;
  for (const char* p = in ? in : ""; *p; p++) {
    char c = lower(*p);
    if (c == ' ' || c == '\t') {
      space = k > 0;
      continue;
    }
    if (!alnum(c) && !strchr("+-_.#", c)) continue;
    if (space) {
      if (k >= BUTTON_LEN) break;
      out[k++] = ' ';
      space = false;
    }
    if (k >= BUTTON_LEN) break;
    out[k++] = c;
  }
  while (k && out[k - 1] == ' ') k--;
  out[k] = 0;
  return k > 0;
}

int nextNumber(const char* const* names, int count, const char* prefix) {
  for (int n = 1;; n++) {
    char want[40];
    snprintf(want, sizeof want, "%s %d", prefix, n);
    bool taken = false;
    for (int i = 0; i < count && !taken; i++) taken = names[i] && !strcmp(names[i], want);
    if (!taken) return n;
  }
}

size_t normalizeQuery(const char* in, char* out, size_t max) {
  if (!out || !max) return 0;
  size_t k = 0;
  bool overflow = false;
  auto put = [&](char c) {
    if (k + 1 >= max) { overflow = true; return; }
    out[k++] = c;
  };
  auto puts = [&](const char* s) { while (*s) put(*s++); };
  out[0] = 0;
  if (!in) return 0;
  while (*in == ' ') in++;
  if (strchr(in, '&') || !strchr(in, ' ')) {  // already a query (or one word)
    const char* eq = strchr(in, '=');
    if (!eq && *in) puts("op=");
    puts(in);
    out[k] = 0;
    return overflow ? 0 : k;
  }
  bool first = true, inValue = false;
  const char* p = in;
  while (*p) {
    while (*p == ' ') p++;
    if (!*p) break;
    const char* e = strchr(p, ' ');
    const size_t len = e ? (size_t)(e - p) : strlen(p);
    const char* eq = (const char*)memchr(p, '=', len);
    if (eq) {
      if (!first) put('&');
      for (size_t i = 0; i < len; i++) put(p[i]);
      inValue = true;
    } else if (inValue) {  // "btn=volume up": the value goes on
      puts("%20");
      for (size_t i = 0; i < len; i++) put(p[i]);
    } else if (first) {
      puts("op=");
      for (size_t i = 0; i < len; i++) put(p[i]);
    } else {
      put('&');  // a stray word: a flag ("once")
      for (size_t i = 0; i < len; i++) put(p[i]);
      puts("=1");
    }
    first = false;
    p += len;
  }
  out[k] = 0;
  return overflow ? 0 : k;
}

}  // namespace irnames

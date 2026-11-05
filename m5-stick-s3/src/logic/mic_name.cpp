#include "mic_name.h"

#include <stdio.h>

namespace micname {

void slug(const char* name, char* out, size_t n) {
  if (!n) return;
  size_t len = 0;
  bool dash = false;
  for (const char* p = name ? name : ""; *p && len + 1 < n; p++) {
    char c = *p;
    if (c >= 'A' && c <= 'Z') c = c - 'A' + 'a';
    if ((c >= 'a' && c <= 'z') || (c >= '0' && c <= '9')) {
      out[len++] = c;
      dash = false;
    } else if (len && !dash) {
      out[len++] = '-';
      dash = true;
    }
  }
  while (len && out[len - 1] == '-') len--;
  out[len] = 0;
}

void usb(const char* stickName, char* out, size_t n) {
  char s[32];
  slug(stickName, s, sizeof s);
  snprintf(out, n, "%s-mic-usb", *s ? s : "stick");
}

}  // namespace micname

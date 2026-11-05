// The stick's microphone names, after the stick's own name so two sticks' mics differ:
// "Arbaaz StickS3" -> "arbaaz-sticks3-usb-mic" (the Mac agent names the Bluetooth one
// "arbaaz-sticks3-bt-mic" with the same rule).
#pragma once
#include <stddef.h>

namespace micname {

// Lower-case letters and digits, anything else -> one "-", none at the ends.
void slug(const char* name, char* out, size_t n);

// "<slug>-usb-mic", or "stickybot-usb-mic" when the name has nothing usable.
void usb(const char* stickName, char* out, size_t n);

}  // namespace micname

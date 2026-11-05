// The stick's microphone names, after the stick's own name so two sticks' mics differ:
// "Arbaaz StickS3" -> "arbaaz-sticks3-mic-usb" (the Mac agent names the Bluetooth one
// "arbaaz-sticks3-mic-bluetooth" with the same rule).
#pragma once
#include <stddef.h>

namespace micname {

// Lower-case letters and digits, anything else -> one "-", none at the ends.
void slug(const char* name, char* out, size_t n);

// "<slug>-mic-usb", or "stick-mic-usb" when the name has nothing usable.
void usb(const char* stickName, char* out, size_t n);

}  // namespace micname

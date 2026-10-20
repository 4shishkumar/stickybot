// Text layout for the screens: greedy word wrap and "..." cutting, with the
// width measured by a callback (the real font on the stick, a fake in tests).
#pragma once
#include <string>
#include <vector>

namespace text {

using Width = int (*)(const std::string& s, void* ctx);

// One line no wider than maxW, cut with "..." if needed.
std::string ellipsize(const std::string& s, int maxW, Width w, void* ctx);
// At most maxLines lines; if the text doesn't fit, the last line ends in "...".
std::vector<std::string> wrap(const std::string& s, int maxLines, int maxW, Width w, void* ctx);

}  // namespace text

#include "protocol.h"
#include "logic/ir_names.h"
#include "logic/button_map.h"
#include <cstdlib>
#include <string>

namespace protocol {

static String urlDecode(const String& s) {
  String o;
  for (size_t i = 0; i < s.length(); i++) {
    char c = s[i];
    if (c == '+') o += ' ';
    else if (c == '%' && i + 2 < s.length()) {
      o += (char)strtol(s.substring(i + 1, i + 3).c_str(), nullptr, 16);
      i += 2;
    } else o += c;
  }
  return o;
}

// "a=1&b=x%20y" -> value of `name`
static String queryArg(const String& query, const char* name) {
  String key = String(name) + "=";
  int start = 0;
  while (start < (int)query.length()) {
    int amp = query.indexOf('&', start);
    if (amp < 0) amp = query.length();
    if (query.substring(start, amp).startsWith(key))
      return urlDecode(query.substring(start + key.length(), amp));
    start = amp + 1;
  }
  return "";
}

// Mapping writes are one four-row transaction. Reject ambiguous query fields or
// damaged escapes before Arduino String or a transport can truncate their values.
static bool buttonQuery(const String& query) {
  if (query.length() > buttons::COUNT * (4 + buttons::MAX_BINDING_BYTES * 3)) return false;
  auto hex = [](char c) {
    return (c >= '0' && c <= '9') || (c >= 'A' && c <= 'F') || (c >= 'a' && c <= 'f');
  };
  unsigned seen = 0;
  for (size_t start = 0; start < query.length();) {
    int amp = query.indexOf('&', start);
    size_t end = amp < 0 ? query.length() : (size_t)amp;
    if (end <= start + 3 || query[start + 2] != '=') return false;
    String key = query.substring(start, start + 2);
    unsigned bit = key == "b2" ? 1 : key == "b3" ? 2 : key == "ab" ? 4 : key == "bh" ? 8 : 0;
    if (!bit || (seen & bit)) return false;
    seen |= bit;
    for (size_t i = start + 3; i < end; ++i) {
      if (query[i] != '%') continue;
      if (i + 2 >= end || !hex(query[i + 1]) || !hex(query[i + 2]) ||
          (query[i + 1] == '0' && query[i + 2] == '0')) return false;
      i += 2;
    }
    if (amp >= 0 && end + 1 == query.length()) return false;
    start = end + 1;
  }
  return seen == 15;
}

Result runLine(const String& line, Source source) {
  if (!line.length() || line[0] != '@') {
    Result r;
    r.status = 400;
    r.json = "{\"ok\":false,\"error\":\"expected @action\"}";
    return r;
  }
  int sp = line.indexOf(' ');
  String action = line.substring(1, sp < 0 ? line.length() : sp);
  String query = sp < 0 ? "" : line.substring(sp + 1);
  if (action == "buttons" && !buttonQuery(query)) return {400, "{\"ok\":false,\"error\":\"Invalid button map\"}"};
  if (action == "ir") {  // "@ir send dev=tv btn=volume up" -> op=send&dev=tv&btn=volume%20up
    std::string q(query.length() * 3 + 16, '\0');
    if (irnames::normalizeQuery(query.c_str(), &q[0], q.size()) || !query.length()) query = q.c_str();
  }
  // Check the decoded value used by the executor, before any settings mutation.
  // Enrollment never authorizes a Mac to erase saved hosts remotely.
  if (source == Source::Bluetooth && action == "hosts" && queryArg(query, "forget").length())
    return {403, "{\"ok\":false,\"error\":\"Forget saved Macs on the stick or over USB\"}"};
  return run(action, [&](const char* name) { return queryArg(query, name); });
}

}  // namespace protocol

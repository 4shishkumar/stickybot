#include "protocol.h"
#include "logic/ir_names.h"
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

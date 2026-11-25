#include "protocol.h"
#include "logic/ir_names.h"
#include "logic/button_map.h"
#include <cstdlib>
#include <cctype>
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

static bool validEscapes(const String& query, size_t start, size_t end) {
  for (size_t i = start; i < end; ++i) {
    if (query[i] != '%') continue;
    if (i + 2 >= end || !std::isxdigit((unsigned char)query[i + 1]) || !std::isxdigit((unsigned char)query[i + 2]) ||
        (query[i + 1] == '0' && query[i + 2] == '0')) return false;
    i += 2;
  }
  return true;
}

// Mapping writes are one four-row transaction. Reject ambiguous query fields or
// damaged escapes before Arduino String or a transport can truncate their values.
static bool buttonQuery(const String& query) {
  if (query.length() > buttons::COUNT * (4 + buttons::MAX_BINDING_BYTES * 3)) return false;
  unsigned seen = 0;
  for (size_t start = 0; start < query.length();) {
    int amp = query.indexOf('&', start);
    size_t end = amp < 0 ? query.length() : (size_t)amp;
    if (end <= start + 3 || query[start + 2] != '=') return false;
    String key = query.substring(start, start + 2);
    unsigned bit = key == "b2" ? 1 : key == "b3" ? 2 : key == "ab" ? 4 : key == "bh" ? 8 : 0;
    if (!bit || (seen & bit)) return false;
    seen |= bit;
    if (!validEscapes(query, start + 3, end)) return false;
    if (amp >= 0 && end + 1 == query.length()) return false;
    start = end + 1;
  }
  return seen == 15;
}

// Empty and absent favourites mean different things: "-" clears, omission preserves.
static bool launcherQuery(const String& query) {
  if (query.length() > 512) return false;
  unsigned seen = 0;
  for (size_t start = 0; start < query.length();) {
    const int amp = query.indexOf('&', start);
    const size_t end = amp < 0 ? query.length() : (size_t)amp;
    const int eq = query.indexOf('=', start);
    if (eq < 0 || (size_t)eq + 1 >= end) return false;
    String key = query.substring(start, eq);
    unsigned bit = key == "tiles" ? 1 : key == "favorites" ? 2 : key == "reset" ? 4 : 0;
    if (!bit || (seen & bit)) return false;
    seen |= bit;
    if (!validEscapes(query, eq + 1, end)) return false;
    if (amp >= 0 && end + 1 == query.length()) return false;
    start = end + 1;
  }
  return !(seen & 4) || seen == 4;
}

Result runLine(const String& line, Source source, OpenTile openTile) {
  if (!line.length() || line[0] != '@') {
    Result r;
    r.status = 400;
    r.json = "{\"ok\":false,\"error\":\"expected @action\"}";
    return r;
  }
  int sp = line.indexOf(' ');
  String action = line.substring(1, sp < 0 ? line.length() : sp);
  String query = sp < 0 ? "" : line.substring(sp + 1);
  if (action == "stick-open") {
    if (source != Source::Bluetooth || !openTile)
      return {403, "{\"ok\":false,\"error\":\"Select this Mac on the stick first\"}"};
    // An exact full tile ID is the entire argument. No command strings, aliases or extra fields.
    launcher::Tile tile;
    if (!query.startsWith("app=") || query.length() <= 5 ||
        !launcher::fromName(query.c_str() + 4, query.length() - 4, tile))
      return {400, "{\"ok\":false,\"error\":\"Invalid stick app\"}"};
    if (!openTile(tile)) return {409, "{\"ok\":false,\"error\":\"Stick is busy\"}"};
    return {200, String("{\"ok\":true,\"app\":\"") + launcher::info(tile).id + "\"}"};
  }
  if (action == "launcher" && source == Source::BackgroundBluetooth && query.length())
    return {403, "{\"ok\":false,\"error\":\"Select this Mac on the stick first\"}"};
  if (action == "launcher" && !launcherQuery(query)) return {400, "{\"ok\":false,\"error\":\"Invalid launcher settings\"}"};
  if (action == "buttons" && !buttonQuery(query)) return {400, "{\"ok\":false,\"error\":\"Invalid button map\"}"};
  if (action == "ir") {  // "@ir send dev=tv btn=volume up" -> op=send&dev=tv&btn=volume%20up
    std::string q(query.length() * 3 + 16, '\0');
    if (irnames::normalizeQuery(query.c_str(), &q[0], q.size()) || !query.length()) query = q.c_str();
  }
  // Check the decoded value used by the executor, before any settings mutation.
  // Enrollment never authorizes a Mac to erase saved hosts remotely.
  if (source != Source::Serial && action == "hosts" && queryArg(query, "forget").length())
    return {403, "{\"ok\":false,\"error\":\"Forget saved Macs on the stick or over USB\"}"};
  return run(action, [&](const char* name) { return queryArg(query, name); });
}

}  // namespace protocol

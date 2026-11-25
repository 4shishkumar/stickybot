#include <unity.h>
#include <string>
// Exercise the actual command parser and transport gate; only the hardware
// executor is substituted, so a forbidden dispatch is observable without NVS.
#include "../../src/protocol_line.cpp"

static int executions;
static std::string action, forget, shared, button;
namespace protocol {
Result run(const String& name, const Args& args) {
  ++executions;
  action = name.c_str();
  forget = args("forget").c_str();
  shared = args("shared").c_str();
  button = args("btn").c_str();
  return {200, "{\"ok\":true}", !forget.empty()};
}
}
void setUp() { executions = 0; action.clear(); forget.clear(); shared.clear(); button.clear(); }
void tearDown() {}

void bluetooth_cannot_forget_any_host_or_change_mode_in_the_same_command() {
  for (const char* line : {"@hosts forget=all", "@hosts forget=test-host", "@hosts forget=%61ll",
       "@hosts shared=0&forget=all", "@hosts forget=+"}) {
    auto result = protocol::runLine(line, protocol::Source::Bluetooth);
    TEST_ASSERT_EQUAL(403, result.status);
    TEST_ASSERT_FALSE(result.restart);
    TEST_ASSERT_EQUAL(0, executions);
  }
}
void serial_can_forget_one_or_all_hosts() {
  for (const char* line : {"@hosts forget=all", "@hosts forget=%61ll", "@hosts forget=test-host"}) {
    auto result = protocol::runLine(line, protocol::Source::Serial);
    TEST_ASSERT_EQUAL(200, result.status);
    TEST_ASSERT_TRUE(result.restart);
    TEST_ASSERT_EQUAL_STRING("hosts", action.c_str());
    TEST_ASSERT_FALSE(forget.empty());
  }
  TEST_ASSERT_EQUAL(3, executions);
}
void bluetooth_host_queries_and_mode_changes_still_dispatch() {
  for (const char* line : {"@hosts", "@hosts shared=1", "@hosts forget=", "@state"}) {
    auto result = protocol::runLine(line, protocol::Source::Bluetooth);
    TEST_ASSERT_EQUAL(200, result.status);
    TEST_ASSERT_FALSE(result.restart);
  }
  TEST_ASSERT_EQUAL(4, executions);
}
void malformed_commands_never_dispatch_and_ir_keeps_its_normalization() {
  TEST_ASSERT_EQUAL(400, protocol::runLine("hosts forget=all", protocol::Source::Bluetooth).status);
  TEST_ASSERT_EQUAL(0, executions);
  TEST_ASSERT_EQUAL(200, protocol::runLine("@ir send dev=tv btn=volume up", protocol::Source::Bluetooth).status);
  TEST_ASSERT_EQUAL_STRING("ir", action.c_str());
  TEST_ASSERT_EQUAL_STRING("volume up", button.c_str());
}
void mapping_writes_require_one_unambiguous_value_per_gesture() {
  for (const char* line : {"@buttons", "@buttons b2=unlock&b3=launcher&ab=none",
       "@buttons b2=unlock&b2=none&b3=talk&ab=reply&bh=talk",
       "@buttons b2=app:abc%00def&b3=launcher&ab=none&bh=talk",
       "@buttons b2=app:x%ZZ&b3=launcher&ab=none&bh=talk",
       "@buttons b2=unlock&b3=launcher&ab=none&bh=talk&"}) {
    TEST_ASSERT_EQUAL(400, protocol::runLine(line, protocol::Source::Bluetooth).status);
  }
  TEST_ASSERT_EQUAL(0, executions);
  TEST_ASSERT_EQUAL(200, protocol::runLine("@buttons bh=talk&b3=app%3AMy%20App&ab=reply&b2=launcher",
                                          protocol::Source::Bluetooth).status);
}
void launcher_query_preserves_absent_and_rejects_ambiguous_favorites() {
  using protocol::Source;
  for (const char* line : {"@launcher", "@launcher reset=1", "@launcher tiles=air,remote",
       "@launcher favorites=-", "@launcher tiles=air%2Cremote&favorites=air"}) {
    TEST_ASSERT_EQUAL(200, protocol::runLine(line, Source::Bluetooth).status);
  }
  for (const char* line : {"@launcher tiles=air&favorites=", "@launcher favorites=air%00remote",
       "@launcher favorites=air%G1", "@launcher reset=1&favorites=air", "@launcher favorites=air&favorites=-"}) {
    TEST_ASSERT_EQUAL(400, protocol::runLine(line, Source::Bluetooth).status);
  }
  TEST_ASSERT_EQUAL(5, executions);
}

static int opens;
static launcher::Tile opened;
static bool open(launcher::Tile tile) { ++opens; opened = tile; return true; }

void stick_open_is_single_destination_and_selected_ble_only() {
  using protocol::Source;
  opens = 0;
  for (int i = 0; i < launcher::TILE_COUNT; ++i) {
    const char* id = launcher::info((launcher::Tile)i).id;
    auto r = protocol::runLine(String("@stick-open app=") + id, Source::Bluetooth, open);
    TEST_ASSERT_EQUAL(200, r.status);
    TEST_ASSERT_EQUAL(i, (int)opened);
  }
  for (const char* line : {"@stick-open", "@stick-open app=g", "@stick-open app=Terminal",
       "@stick-open app=games&app=air", "@stick-open app=air%00", "@stick-open app=air extra"}) {
    TEST_ASSERT_EQUAL(400, protocol::runLine(line, Source::Bluetooth, open).status);
  }
  TEST_ASSERT_EQUAL(403, protocol::runLine("@launcher tiles=air&favorites=air", Source::BackgroundBluetooth).status);
  TEST_ASSERT_EQUAL(403, protocol::runLine("@stick-open app=air", Source::BackgroundBluetooth, open).status);
  TEST_ASSERT_EQUAL(403, protocol::runLine("@stick-open app=air", Source::Serial, open).status);
  TEST_ASSERT_EQUAL(403, protocol::runLine("@stick-open app=air", Source::Bluetooth).status);
  auto busy = [](launcher::Tile) { return false; };
  TEST_ASSERT_EQUAL(409, protocol::runLine("@stick-open app=air", Source::Bluetooth, busy).status);
  TEST_ASSERT_EQUAL(launcher::TILE_COUNT, opens);
  TEST_ASSERT_EQUAL(0, executions);
}

int main(int, char**) {
  UNITY_BEGIN();
  RUN_TEST(stick_open_is_single_destination_and_selected_ble_only);
  RUN_TEST(launcher_query_preserves_absent_and_rejects_ambiguous_favorites);
  RUN_TEST(bluetooth_cannot_forget_any_host_or_change_mode_in_the_same_command);
  RUN_TEST(mapping_writes_require_one_unambiguous_value_per_gesture);
  RUN_TEST(serial_can_forget_one_or_all_hosts);
  RUN_TEST(bluetooth_host_queries_and_mode_changes_still_dispatch);
  RUN_TEST(malformed_commands_never_dispatch_and_ir_keeps_its_normalization);
  return UNITY_END();
}

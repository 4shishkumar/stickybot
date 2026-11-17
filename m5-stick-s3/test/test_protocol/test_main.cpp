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
int main(int, char**) {
  UNITY_BEGIN();
  RUN_TEST(bluetooth_cannot_forget_any_host_or_change_mode_in_the_same_command);
  RUN_TEST(serial_can_forget_one_or_all_hosts);
  RUN_TEST(bluetooth_host_queries_and_mode_changes_still_dispatch);
  RUN_TEST(malformed_commands_never_dispatch_and_ir_keeps_its_normalization);
  return UNITY_END();
}

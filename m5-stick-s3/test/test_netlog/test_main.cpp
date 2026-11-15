#include <unity.h>
#include <array>
#include <string>
#include <WiFi.h>
// Compile the actual TeeLog implementation and TCP command dispatcher. Only the
// hardware Print/WiFi sinks and command execution are substituted below.
#include "../../src/netlog.cpp"

static const char* PRIVATE_JSON = "{\"ok\":true,\"name\":\"private-marker-host\",\"value\":\"private-marker-data\"}";
namespace protocol {
void serialLine(String line, Print& reply) {
  // Command execution is outside this transport test; the real protocol's wire
  // formatter and the real netlog requester selection run without substitution.
  if (line == "@private") machineReply(reply, PRIVATE_JSON);
  else if (line == "set") humanReply(reply, PRIVATE_JSON);
  else humanReply(reply, "private-marker-human-response");
}
}
static std::array<std::shared_ptr<FakeConnection>, 3> clients;
void setUp() {
  WiFi.mode = 0;
  netlog::loop();
  Serial.output.clear();
  WiFi.mode = WIFI_STA;
  for (auto& c : clients) {
    c = std::make_shared<FakeConnection>();
    WiFiServer::pending.push_back(c);
    netlog::loop();
  }
  Log.write((uint8_t)'\n');  // end a previous test's deliberately partial diagnostic
  Serial.output.clear();
  for (auto& c : clients) c->output.clear();
}
void tearDown() {
  WiFi.mode = 0;
  netlog::loop();
}
static void noPrivateDiagnostics() {
  TEST_ASSERT_EQUAL(std::string::npos, Serial.output.find("private-marker"));
  for (const auto& c : clients)
    TEST_ASSERT_EQUAL(std::string::npos, c->output.find("private-marker"));
}
void test_real_teelog_rejects_private_print_printf_and_raw_fragments_on_all_sinks() {
  const char* fields[] = {"host-label", "host-name", "prompt", "question", "option-a", "option-b", "hint",
    "origin", "event-title", "device-name", "wifi-ssid", "ir-label", "ir-code", "face-id", "face-version",
    "validation-error", "ota-version", "filename", "bond-address", "pairing-code", "key-report"};
  for (auto field : fields) {
    std::string privateValue = "private-marker-" + std::string(field);
    Log.print(privateValue.c_str());
    Log.println(String(privateValue));
    Log.printf("unsafe %s\n", privateValue.c_str());
    for (unsigned char c : privateValue) Log.write(c);
    Log.write((uint8_t)'\n');
    Log.write((const uint8_t*)privateValue.data(), privateValue.size());
    Log.println();
  }
  noPrivateDiagnostics();
  TEST_ASSERT_NOT_EQUAL(std::string::npos, Serial.output.find("diagnostic payload omitted"));
  for (const auto& c : clients) TEST_ASSERT_EQUAL_STRING(Serial.output.c_str(), c->output.c_str());
}
void test_fixed_events_reach_usb_and_tcp_without_payload_or_protocol_prefix() {
  for (unsigned i = 0; i < (unsigned)diagnostics::Event::Count; ++i) Log.event((diagnostics::Event)i);
  noPrivateDiagnostics();
  TEST_ASSERT_NOT_EQUAL(std::string::npos, Serial.output.find("ready device\r\n"));
  TEST_ASSERT_NOT_EQUAL(std::string::npos, Serial.output.find("Request shown on the stick\r\n"));
  TEST_ASSERT_EQUAL(std::string::npos, Serial.output.find('@'));
  for (const auto& c : clients) TEST_ASSERT_EQUAL_STRING(Serial.output.c_str(), c->output.c_str());
}
void test_typed_mechanical_counters_preserve_values_without_private_text() {
  Log.count(diagnostics::Counter::FirmwareBytes, 12345);
  Log.count(diagnostics::Counter::AudioDropped, 2);
  TEST_ASSERT_EQUAL_STRING("Firmware image bytes: 12345\r\nAudio frames dropped: 2\r\n", Serial.output.c_str());
  noPrivateDiagnostics();
  for (const auto& c : clients) TEST_ASSERT_EQUAL_STRING(Serial.output.c_str(), c->output.c_str());
}
void test_usb_machine_response_keeps_legacy_frame_and_never_reaches_tcp() {
  protocol::serialLine("@private", Serial);
  std::string expected = "@" + std::string(PRIVATE_JSON) + "\r\n";
  TEST_ASSERT_EQUAL_STRING(expected.c_str(), Serial.output.c_str());
  for (const auto& c : clients) TEST_ASSERT_TRUE(c->output.empty());
}
void test_human_success_reply_is_explicit_and_only_goes_to_requester() {
  protocol::serialLine("set", Serial);
  std::string expected = std::string(PRIVATE_JSON) + "\r\n";
  TEST_ASSERT_EQUAL_STRING(expected.c_str(), Serial.output.c_str());
  for (const auto& c : clients) TEST_ASSERT_TRUE(c->output.empty());
  Serial.output.clear();
  clients[0]->input = "set\n";
  netlog::loop();
  TEST_ASSERT_EQUAL_STRING(expected.c_str(), clients[0]->output.c_str());
  TEST_ASSERT_TRUE(Serial.output.empty());
  TEST_ASSERT_TRUE(clients[1]->output.empty());
  TEST_ASSERT_TRUE(clients[2]->output.empty());
}
void test_tcp_machine_response_reaches_only_its_requesting_client() {
  clients[1]->input = "@private\n";
  netlog::loop();
  std::string expected = "@" + std::string(PRIVATE_JSON) + "\r\n";
  TEST_ASSERT_EQUAL_STRING(expected.c_str(), clients[1]->output.c_str());
  TEST_ASSERT_TRUE(Serial.output.empty());
  TEST_ASSERT_TRUE(clients[0]->output.empty());
  TEST_ASSERT_TRUE(clients[2]->output.empty());
}
void test_tcp_human_response_is_requester_only_while_events_still_fan_out() {
  clients[2]->input = "show\r\n";
  netlog::loop();
  TEST_ASSERT_EQUAL_STRING("private-marker-human-response\r\n", clients[2]->output.c_str());
  TEST_ASSERT_TRUE(Serial.output.empty());
  TEST_ASSERT_TRUE(clients[0]->output.empty());
  TEST_ASSERT_TRUE(clients[1]->output.empty());
  Log.event(diagnostics::Event::Hid);
  TEST_ASSERT_EQUAL_STRING("HID state changed\r\n", Serial.output.c_str());
  TEST_ASSERT_EQUAL_STRING(Serial.output.c_str(), clients[0]->output.c_str());
  TEST_ASSERT_EQUAL_STRING(Serial.output.c_str(), clients[1]->output.c_str());
}
int main(int, char**) {
  UNITY_BEGIN();
  RUN_TEST(test_real_teelog_rejects_private_print_printf_and_raw_fragments_on_all_sinks);
  RUN_TEST(test_fixed_events_reach_usb_and_tcp_without_payload_or_protocol_prefix);
  RUN_TEST(test_typed_mechanical_counters_preserve_values_without_private_text);
  RUN_TEST(test_usb_machine_response_keeps_legacy_frame_and_never_reaches_tcp);
  RUN_TEST(test_human_success_reply_is_explicit_and_only_goes_to_requester);
  RUN_TEST(test_tcp_machine_response_reaches_only_its_requesting_client);
  RUN_TEST(test_tcp_human_response_is_requester_only_while_events_still_fan_out);
  return UNITY_END();
}

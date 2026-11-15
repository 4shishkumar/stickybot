#include <unity.h>
#include "logic/hosts.h"
using namespace hosts;
void setUp() {}
void tearDown() {}
static Registry pair() {
  Registry r;
  r.hello(1,"bond-a",true,"host-a","Test Laptop","Desk","session-a",10);
  r.hello(2,"bond-b",true,"host-b","Test Desktop","Desk","session-b",10);
  r.select("host-a",false);
  return r;
}
void test_identity_is_bound_to_bond_and_session() {
  auto r = pair();
  TEST_ASSERT_EQUAL(-1,r.hello(3,"bond-c",true,"host-a","Test","Desk","s",20));
  TEST_ASSERT_EQUAL(-1,r.hello(1,"bond-a",true,"other","Test","Desk","s",20));
  TEST_ASSERT_EQUAL(-1,r.hello(3,"bond-c",false,"host-c","Test","Desk","s",20));
  auto l = r.lease(1);
  TEST_ASSERT_TRUE(r.owns(l,1)); TEST_ASSERT_FALSE(r.owns(l,2));
  r.hello(1,"bond-a",true,"host-a","Test Laptop","Desk","new-session",20);
  TEST_ASSERT_FALSE(r.owns(l,1)); TEST_ASSERT_FALSE(r.valid(1,"session-a"));
}
void test_focus_never_follows_reconnect_or_busy_switch() {
  auto r=pair();
  TEST_ASSERT_FALSE(r.select("host-b",true)); TEST_ASSERT_EQUAL(1,r.selected());
  r.disconnect(1); TEST_ASSERT_EQUAL(NONE,r.selected());
  r.reconnect(5,"bond-b"); TEST_ASSERT_EQUAL(NONE,r.selected());
  TEST_ASSERT_TRUE(r.select("host-b",false)); TEST_ASSERT_EQUAL(5,r.selected());
  TEST_ASSERT_FALSE(r.valid(5,"session-b"));
}
void test_names_capacity_and_presence_are_bounded() {
  auto r=pair();
  TEST_ASSERT_NOT_EQUAL(0,r.display(0).compare(r.display(1)));
  TEST_ASSERT_FALSE(label("Longer than twelve")); TEST_ASSERT_FALSE(label("bad\nname"));
  TEST_ASSERT_FALSE(token("fake\"id"));
  TEST_ASSERT_TRUE(r.online(0,100)); TEST_ASSERT_FALSE(r.online(0,30000));
  TEST_ASSERT_EQUAL(2,r.hello(3,"bond-c",true,"host-c","Test","Third","s",1));
  TEST_ASSERT_EQUAL(-1,r.hello(4,"bond-d",true,"host-d","Test","Fourth","s",1));
}
void test_interleaved_lines_and_oversize_discard() {
  Lines a,b; std::string out;
  a.push('a',out); b.push('b',out);
  TEST_ASSERT_TRUE(a.push('\n',out)); TEST_ASSERT_EQUAL_STRING("a",out.c_str());
  TEST_ASSERT_TRUE(b.push('\n',out)); TEST_ASSERT_EQUAL_STRING("b",out.c_str());
  for(int i=0;i<5000;++i) a.push('x',out);
  TEST_ASSERT_FALSE(a.push('\n',out));
  a.push('c',out); TEST_ASSERT_TRUE(a.push('\n',out)); TEST_ASSERT_EQUAL_STRING("c",out.c_str());
}
void test_reused_handle_does_not_revive_recording_or_transfer_owner() {
  auto r = pair(); auto voice = r.lease(1); auto bulk = r.lease(2);
  r.disconnect(1);
  r.hello(1,"bond-c",true,"host-c","Test Tablet","Travel","session-c",30);
  TEST_ASSERT_FALSE(r.owns(voice,1));
  TEST_ASSERT_TRUE(r.owns(bulk,2));
  TEST_ASSERT_EQUAL(NONE,r.selected());
}
void test_rename_preserves_lease_focus_and_record_slot() {
  auto r = pair(); auto owner = r.lease(1);
  TEST_ASSERT_EQUAL(0,r.hello(1,"bond-a",true,"host-a","Renamed Test Laptop","Work","session-a",30));
  TEST_ASSERT_TRUE(r.owns(owner,1)); TEST_ASSERT_EQUAL(1,r.selected());
  TEST_ASSERT_EQUAL_STRING("Work",r.display(0).c_str());
  r.disconnect(1);
  TEST_ASSERT_EQUAL_STRING("Work",r.display(0).c_str());
  TEST_ASSERT_FALSE(r.online(0,40));
  TEST_ASSERT_EQUAL_STRING("host-a",r.active.c_str());
}
void test_unicode_names_are_valid_utf8_bounded_and_not_labels() {
  TEST_ASSERT_TRUE(name("Test \xc3\xa9 Laptop",320));
  TEST_ASSERT_FALSE(label("Test \xc3\xa9"));
  TEST_ASSERT_FALSE(name(std::string(81,'a'),320));
  TEST_ASSERT_FALSE(name("bad\xc0\xaf",320));
  TEST_ASSERT_FALSE(name("bad\xed\xa0\x80",320));
  TEST_ASSERT_FALSE(name("bad\xc2\x85",320));
  TEST_ASSERT_FALSE(name("bad\xf4\x90\x80\x80",320));
  TEST_ASSERT_FALSE(label("1234567890123"));
  TEST_ASSERT_TRUE(label("123456789012"));
}
void test_overflow_cannot_execute_trailing_json() {
  Lines a; std::string out;
  for (int i=0;i<4097;++i) a.push('x',out);
  for (char c : std::string("{\"proto\":\"@macpass pass=bad\"}\n"))
    TEST_ASSERT_FALSE(a.push(c,out));
  bool complete=false;
  for (char c : std::string("{\"proto\":\"@state\"}\n")) complete |= a.push(c,out);
  TEST_ASSERT_TRUE(complete);
  TEST_ASSERT_EQUAL_STRING("{\"proto\":\"@state\"}",out.c_str());
}
void test_transfer_resume_requires_same_bonded_host_and_new_begin() {
  auto r = pair(); Transfer bulk;
  TEST_ASSERT_TRUE(bulk.claim(r,1,true,false));
  Lease before = bulk.owner;
  r.disconnect(1); bulk.revoke();
  TEST_ASSERT_FALSE(r.owns(before,1));
  TEST_ASSERT_FALSE(bulk.claim(r,2,true,true)); // another authenticated Mac cannot take the image
  r.hello(7,"bond-a",true,"host-a","Test Laptop","Desk","new-session",30);
  TEST_ASSERT_FALSE(bulk.claim(r,7,false,true)); // raw data/end must not reclaim a stale session
  TEST_ASSERT_TRUE(bulk.claim(r,7,true,true));
  TEST_ASSERT_TRUE(r.owns(bulk.owner,7));
  TEST_ASSERT_FALSE(r.owns(before,7));
  TEST_ASSERT_EQUAL_STRING("host-a",bulk.host.c_str());
}
int main(int,char**) {
  UNITY_BEGIN();
  RUN_TEST(test_identity_is_bound_to_bond_and_session);
  RUN_TEST(test_focus_never_follows_reconnect_or_busy_switch);
  RUN_TEST(test_names_capacity_and_presence_are_bounded);
  RUN_TEST(test_interleaved_lines_and_oversize_discard);
  RUN_TEST(test_reused_handle_does_not_revive_recording_or_transfer_owner);
  RUN_TEST(test_rename_preserves_lease_focus_and_record_slot);
  RUN_TEST(test_unicode_names_are_valid_utf8_bounded_and_not_labels);
  RUN_TEST(test_overflow_cannot_execute_trailing_json);
  RUN_TEST(test_transfer_resume_requires_same_bonded_host_and_new_begin);
  return UNITY_END();
}

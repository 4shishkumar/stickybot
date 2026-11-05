// Approvals and questions (firmware 1.7.2): what each input answers, and the button labels.
#include <unity.h>
#include <string>
#include "logic/prompt.h"

void setUp() {}
void tearDown() {}

using prompt::Input;

void test_approval_answers() {
  TEST_ASSERT_EQUAL_STRING("once", prompt::decision(false, Input::A));
  TEST_ASSERT_EQUAL_STRING("deny", prompt::decision(false, Input::B));
  TEST_ASSERT_EQUAL_STRING("deny", prompt::decision(false, Input::SHAKE));  // as before 1.7.2
}

void test_question_answers() {
  TEST_ASSERT_EQUAL_STRING("once", prompt::decision(true, Input::A));     // the first option
  TEST_ASSERT_EQUAL_STRING("deny", prompt::decision(true, Input::B));     // the second
  TEST_ASSERT_EQUAL_STRING("dismiss", prompt::decision(true, Input::SHAKE));
}

void test_button_labels() {
  TEST_ASSERT_EQUAL_STRING("A  Staging", prompt::button('A', "Staging", "yes", 16).c_str());
  TEST_ASSERT_EQUAL_STRING("B  no", prompt::button('B', "", "no", 16).c_str());
  TEST_ASSERT_EQUAL_STRING("A  Producti", prompt::button('A', "Production env", "yes", 8).c_str());
}

void test_button_labels_keep_utf8_whole() {
  // "°" is two bytes: cutting at 3 characters keeps it whole
  TEST_ASSERT_EQUAL_STRING("A  28°", prompt::button('A', "28°C", "yes", 3).c_str());
  std::string broken = "ab\xC2";  // ends mid-character
  TEST_ASSERT_EQUAL_STRING("B  ab", prompt::button('B', broken, "no", 16).c_str());
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_approval_answers);
  RUN_TEST(test_question_answers);
  RUN_TEST(test_button_labels);
  RUN_TEST(test_button_labels_keep_utf8_whole);
  return UNITY_END();
}

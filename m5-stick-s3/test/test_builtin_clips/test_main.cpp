// The sound clips built into the firmware (src/sounds_clips.cpp, tools/make_sounds.py) and the
// choice of an event's sound (logic/event_sound.h).
#include <unity.h>
#include <string.h>
#include <string>
#include <vector>
#include "sounds_clips.h"
#include "logic/adpcm.h"
#include "logic/event_sound.h"

void setUp() {}
void tearDown() {}

static const BuiltinClip* byName(const char* n) {
  for (size_t i = 0; i < BUILTIN_CLIP_COUNT; i++)
    if (!strcmp(BUILTIN_CLIPS[i].name, n)) return &BUILTIN_CLIPS[i];
  return nullptr;
}

// Walks a clip's [len u8][frame] records; returns the samples they decode to (-1: malformed).
static long decodeAll(const BuiltinClip& c, int16_t* peak) {
  std::vector<int16_t> buf(1024);
  size_t p = 0;
  long total = 0;
  int16_t pk = 0;
  while (p < c.bytes) {
    size_t n = c.data[p++];
    if (p + n > c.bytes || n <= adpcm::HEADER) return -1;
    size_t got = adpcm::decodeFrame(c.data + p, n, buf.data(), buf.size());
    for (size_t i = 0; i < got; i++) {
      int16_t a = buf[i] < 0 ? (int16_t)(buf[i] == -32768 ? 32767 : -buf[i]) : buf[i];
      if (a > pk) pk = a;
    }
    total += (long)got;
    p += n;
  }
  if (peak) *peak = pk;
  return total;
}

static const char* const CELEBRATIONS[] = {"applause", "clap", "cheer", "party_horn", "tada", "drumroll", "level_up", "coins"};

void test_every_clip_decodes_to_its_length() {
  for (size_t i = 0; i < BUILTIN_CLIP_COUNT; i++) {
    const BuiltinClip& c = BUILTIN_CLIPS[i];
    long n = decodeAll(c, nullptr);
    TEST_ASSERT_TRUE_MESSAGE(n >= 0, c.name);
    // frames hold whole bytes: an odd last frame decodes one sample more
    TEST_ASSERT_TRUE_MESSAGE(n == (long)c.samples || n == (long)c.samples + 1, c.name);
  }
}

void test_names_are_unique() {
  for (size_t i = 0; i < BUILTIN_CLIP_COUNT; i++)
    for (size_t j = i + 1; j < BUILTIN_CLIP_COUNT; j++)
      TEST_ASSERT_TRUE_MESSAGE(strcmp(BUILTIN_CLIPS[i].name, BUILTIN_CLIPS[j].name) != 0, BUILTIN_CLIPS[i].name);
}

void test_celebration_clips_are_there_short_loud_and_small() {
  size_t bytes = 0;
  for (const char* n : CELEBRATIONS) {
    const BuiltinClip* c = byName(n);
    TEST_ASSERT_NOT_NULL_MESSAGE(c, n);
    TEST_ASSERT_TRUE_MESSAGE(c->samples >= 16000 * 6 / 10 && c->samples <= 16000 * 2, n);  // 0.6-2 s
    int16_t peak = 0;
    TEST_ASSERT_TRUE(decodeAll(*c, &peak) > 0);
    TEST_ASSERT_TRUE_MESSAGE(peak > 16000, n);  // made loud like the others
    bytes += c->bytes;
  }
  TEST_ASSERT_TRUE(bytes <= 200 * 1024);
}

void test_the_level_defaults_are_built_in() {
  for (const char* n : {"jingle_win", "confirmation_002", "question_001"}) TEST_ASSERT_NOT_NULL_MESSAGE(byName(n), n);
}

// --- event_sound::play ---------------------------------------------------------------

struct FakeSpeaker {
  std::vector<std::string> tried;
  bool operator()(const char* n) {
    tried.push_back(n);
    return byName(n) != nullptr;
  }
};

void test_event_plays_its_own_sound() {
  FakeSpeaker s;
  TEST_ASSERT_EQUAL_STRING("applause", event_sound::play("applause", "jingle_win", s));
  TEST_ASSERT_EQUAL(1, (int)s.tried.size());
}

void test_empty_sound_plays_the_level_default() {
  FakeSpeaker s;
  TEST_ASSERT_EQUAL_STRING("jingle_win", event_sound::play("", "jingle_win", s));
  TEST_ASSERT_EQUAL(1, (int)s.tried.size());
}

void test_unknown_sound_falls_back_to_the_level_default() {
  FakeSpeaker s;
  TEST_ASSERT_EQUAL_STRING("question_001", event_sound::play("fanfare_from_the_future", "question_001", s));
  TEST_ASSERT_EQUAL(2, (int)s.tried.size());
  TEST_ASSERT_EQUAL_STRING("question_001", s.tried[1].c_str());
}

void test_off_is_silent() {
  FakeSpeaker s;
  TEST_ASSERT_NULL(event_sound::play("off", "jingle_win", s));
  TEST_ASSERT_EQUAL(0, (int)s.tried.size());
}

void test_nothing_known_plays_nothing() {
  FakeSpeaker s;
  TEST_ASSERT_NULL(event_sound::play("nope", "nope_either", s));
  TEST_ASSERT_NULL(event_sound::play(nullptr, nullptr, s));
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_every_clip_decodes_to_its_length);
  RUN_TEST(test_names_are_unique);
  RUN_TEST(test_celebration_clips_are_there_short_loud_and_small);
  RUN_TEST(test_the_level_defaults_are_built_in);
  RUN_TEST(test_event_plays_its_own_sound);
  RUN_TEST(test_empty_sound_plays_the_level_default);
  RUN_TEST(test_unknown_sound_falls_back_to_the_level_default);
  RUN_TEST(test_off_is_silent);
  RUN_TEST(test_nothing_known_plays_nothing);
  return UNITY_END();
}

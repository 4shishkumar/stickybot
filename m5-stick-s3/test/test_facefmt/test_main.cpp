// Face packages' arithmetic: bindings, templates, conditions, tweens, frame rates, the guard rail.
#include <unity.h>
#include <string.h>
#include "logic/facefmt.h"

using namespace facefmt;

void setUp() {}
void tearDown() {}

static Inputs morning() {
  Inputs in;
  in.minute = 9 * 60 + 41;
  in.second = 7;
  in.wday = 5;  // Friday
  in.mday = 6;
  in.mon = 10;  // November
  in.dateShort = "Fri 6 Nov";
  in.dateLong = "Friday 6 November";
  in.name = "StickyBot";
  return in;
}


static const char* text(int id, const Inputs& in) {
  static Value v;
  value(id, in, v);
  return v.s;
}

static const char* tmpl(const char* src, const Inputs& in) {
  static char compiled[96], out[160];
  TEST_ASSERT_TRUE_MESSAGE(compileTemplate(src, compiled, sizeof compiled), src);
  expand(compiled, in, out, sizeof out);
  return out;
}

void test_binding_names() {
  TEST_ASSERT_EQUAL(TIME_HM, bindingIndex("time.hm"));
  TEST_ASSERT_EQUAL(MOTION_TILT_Y, bindingIndex("motion.tiltY"));
  TEST_ASSERT_EQUAL(REACT + EV_MERGED, bindingIndex("react.merged"));
  TEST_ASSERT_EQUAL(REACT + EV_URGENT, bindingIndex("react.urgent"));
  TEST_ASSERT_EQUAL(-1, bindingIndex("react.nope"));
  TEST_ASSERT_EQUAL(-1, bindingIndex("time.hmm"));
  TEST_ASSERT_EQUAL(-1, bindingIndex(""));
  for (int i = 0; i < BIND_COUNT; i++) TEST_ASSERT_EQUAL(i, bindingIndex(bindingName(i)));
  TEST_ASSERT_EQUAL(EV_UPSIDE_DOWN, eventIndex("upside_down"));
  TEST_ASSERT_EQUAL(EV_MERGED, eventForKick(world::Kick::MERGED));
  TEST_ASSERT_EQUAL(EV_LIFT, eventForKick(world::Kick::LIFT));
  TEST_ASSERT_EQUAL(-1, eventForKick(world::Kick::NONE));
}

void test_time_twelve_and_twenty_four() {
  Inputs in = morning();
  TEST_ASSERT_EQUAL_STRING("9:41", text(TIME_HM, in));
  TEST_ASSERT_EQUAL_STRING("AM", text(TIME_AMPM, in));
  in.minute = 21 * 60 + 5;
  TEST_ASSERT_EQUAL_STRING("9:05", text(TIME_HM, in));
  TEST_ASSERT_EQUAL_STRING("21:05", text(TIME_HM24, in));
  TEST_ASSERT_EQUAL_STRING("09", text(TIME_HH12, in));
  TEST_ASSERT_EQUAL_STRING("21", text(TIME_HH24, in));
  TEST_ASSERT_EQUAL_STRING("PM", text(TIME_AMPM, in));
  in.clock24 = true;  // the setting: time.hm follows it
  TEST_ASSERT_EQUAL_STRING("21:05", text(TIME_HM, in));
  in.minute = 0;
  TEST_ASSERT_EQUAL_STRING("0:00", text(TIME_HM, in));
  in.clock24 = false;
  TEST_ASSERT_EQUAL_STRING("12:00", text(TIME_HM, in));
  TEST_ASSERT_EQUAL_STRING("12", text(TIME_H12, in));
  TEST_ASSERT_EQUAL_STRING("midnight", text(TIME_WORDS, in));
  TEST_ASSERT_EQUAL_STRING("07", text(TIME_SS, in));
}

void test_unknown_time_is_empty_and_false() {
  Inputs in;
  Value v;
  value(TIME_HM, in, v);
  TEST_ASSERT_FALSE(v.known);
  TEST_ASSERT_EQUAL_STRING("", v.s);
  value(TIME_KNOWN, in, v);
  TEST_ASSERT_TRUE(v.known);
  TEST_ASSERT_FALSE(v.truthy());
  value(DATE_WEEKDAY, in, v);
  TEST_ASSERT_FALSE(v.truthy());
  value(SKY_DAY, in, v);
  TEST_ASSERT_FALSE(v.known);
  value(DEV_BATTERY, in, v);  // -1: unknown
  TEST_ASSERT_FALSE(v.known);
}

void test_date_parts_from_the_clock() {
  Inputs in = morning();
  TEST_ASSERT_EQUAL_STRING("Friday", text(DATE_WEEKDAY, in));
  TEST_ASSERT_EQUAL_STRING("Fri", text(DATE_DOW, in));
  TEST_ASSERT_EQUAL_STRING("6", text(DATE_DAY, in));
  TEST_ASSERT_EQUAL_STRING("06", text(DATE_DD, in));
  TEST_ASSERT_EQUAL_STRING("November", text(DATE_MONTH, in));
  TEST_ASSERT_EQUAL_STRING("Nov", text(DATE_MON, in));
  TEST_ASSERT_EQUAL_STRING("11", text(DATE_MM, in));
  TEST_ASSERT_EQUAL_STRING("Friday 6 November", text(DATE_LONG, in));
  in.wday = 9;  // out of range: unknown, never an index past the table
  TEST_ASSERT_EQUAL_STRING("", text(DATE_WEEKDAY, in));
}

void test_world_and_pet() {
  Inputs in = morning();
  world::Snapshot w;
  w.valid = true;
  w.merged = 41;
  w.mergedToday = 3;
  w.ci = world::Ci::RED;
  w.xp = 45;  // level 2 (20 XP), 25 into the 30 that level 2 needs
  w.acc = 2;
  in.world = &w;
  TEST_ASSERT_EQUAL_STRING("red", text(WORK_CI, in));
  TEST_ASSERT_EQUAL_STRING("3", text(WORK_MERGED_TODAY, in));
  TEST_ASSERT_EQUAL_STRING("2", text(PET_LEVEL, in));
  TEST_ASSERT_EQUAL_STRING("25", text(PET_INTO, in));
  TEST_ASSERT_EQUAL_STRING("0", text(PET_HAT, in));
  TEST_ASSERT_EQUAL_STRING("1", text(PET_SHADES, in));
  Value v;
  value(WORK_ASKS, in, v);  // the agent isn't linked: work.* isn't known
  TEST_ASSERT_FALSE(v.known);
  in.agentLinked = true;
  value(WORK_ASKS, in, v);
  TEST_ASSERT_TRUE(v.known);
}

void test_templates() {
  Inputs in = morning();
  TEST_ASSERT_EQUAL_STRING("9:41", tmpl("{time.hm}", in));
  TEST_ASSERT_EQUAL_STRING("FRI 06", tmpl("{date.dow:upper} {date.day:02}", in));
  TEST_ASSERT_EQUAL_STRING("007 s", tmpl("{time.second:03} s", in));
  TEST_ASSERT_EQUAL_STRING("friday", tmpl("{date.weekday:lower}", in));
  TEST_ASSERT_EQUAL_STRING("24°", tmpl("{wx.temp}°", (in.wxValid = true, in.wxTemp = 24, in)));
  in.minute = -1;
  TEST_ASSERT_EQUAL_STRING("[]", tmpl("[{time.mm:02}]", in));  // unknown: not padded
  char out[8];
  TEST_ASSERT_FALSE(compileTemplate("{time.nope}", out, sizeof out));
  TEST_ASSERT_FALSE(compileTemplate("{time.hm:05}", out, sizeof out));
  TEST_ASSERT_FALSE(compileTemplate("{time.hm", out, sizeof out));
  TEST_ASSERT_FALSE(compileTemplate("a\x01", out, sizeof out));
  const char* err = nullptr;
  TEST_ASSERT_FALSE(compileTemplate("far too long for eight", out, sizeof out, &err));
  TEST_ASSERT_EQUAL_STRING("too long", err);
}

void test_expand_never_overflows() {
  Inputs in = morning();
  in.agentLinked = true;
  in.cardA = "A very long card line from the agent that goes on and on and on past every buffer";
  char compiled[64], out[16];
  TEST_ASSERT_TRUE(compileTemplate("{card.a}{card.a}", compiled, sizeof compiled));
  TEST_ASSERT_EQUAL(15, expand(compiled, in, out, sizeof out));
  TEST_ASSERT_EQUAL(15, strlen(out));
}

void test_template_uses() {
  char c[32];
  compileTemplate("{time.hm}:{time.ss}", c, sizeof c);
  TEST_ASSERT_TRUE(templateUses(c, secondsBinding));
  compileTemplate("{time.hm}", c, sizeof c);
  TEST_ASSERT_FALSE(templateUses(c, secondsBinding));
}

void test_numbers() {
  Inputs in = morning();
  in.tiltX = 0.5f;
  Num n;
  n.bind = MOTION_TILT_X;
  n.mul = -3;
  TEST_ASSERT_EQUAL_FLOAT(-1.5f, numValue(n, in));
  n.hasLo = true;
  n.lo = -1;
  TEST_ASSERT_EQUAL_FLOAT(-1, numValue(n, in));
  Num k;
  k.k = 12;
  TEST_ASSERT_EQUAL_FLOAT(12, numValue(k, in));
  Frac f;
  f.bind = DEV_BATTERY;
  f.of = 100;
  in.battery = 76;
  TEST_ASSERT_EQUAL_FLOAT(0.76f, fracValue(f, in));
  in.battery = -1;
  TEST_ASSERT_EQUAL_FLOAT(0, fracValue(f, in));
}

void test_conditions() {
  Inputs in = morning();
  world::Snapshot w;
  w.valid = true;
  w.ci = world::Ci::RED;
  in.world = &w;
  Cond c[6];
  // all: [ time.known, {work.ci eq "red"}, not: sky.night ]
  c[0].op = Op::ALL; c[0].n = 3; c[0].first = 1;
  c[1].op = Op::TRUTHY; c[1].bind = TIME_KNOWN;
  c[2].op = Op::EQ; c[2].bind = WORK_CI; c[2].isStr = true; c[2].str = "red";
  c[3].op = Op::NOT; c[3].n = 1; c[3].first = 4;
  c[4].op = Op::TRUTHY; c[4].bind = SKY_NIGHT;
  TEST_ASSERT_TRUE(eval(c, 0, in));
  in.minute = 23 * 60;
  TEST_ASSERT_FALSE(eval(c, 0, in));
  // numbers, and unknown values (only "ne" holds)
  Cond g;
  g.op = Op::GT; g.bind = DEV_BATTERY; g.num = 20;
  in.battery = 76;
  TEST_ASSERT_TRUE(eval(&g, 0, in));
  in.battery = -1;
  TEST_ASSERT_FALSE(eval(&g, 0, in));
  g.op = Op::NE;
  TEST_ASSERT_TRUE(eval(&g, 0, in));
  g.op = Op::FALSY; g.bind = DEV_CHARGING;
  TEST_ASSERT_TRUE(eval(&g, 0, in));
}

void test_condition_depth_is_bounded() {
  Inputs in = morning();
  Cond c[6];
  for (int i = 0; i < 5; i++) { c[i].op = Op::NOT; c[i].n = 1; c[i].first = i + 1; }
  c[5].op = Op::TRUTHY; c[5].bind = TIME_KNOWN;
  TEST_ASSERT_FALSE(eval(c, 0, in));  // deeper than 3: false, whatever it says
}

void test_reactions() {
  Inputs in = morning();
  uint32_t at[EV_COUNT] = {0}, ms[EV_COUNT];
  for (auto& m : ms) m = 3000;
  in.eventAt = at;
  in.reactMs = ms;
  in.now = 10000;
  TEST_ASSERT_EQUAL_STRING("0", text(REACT + EV_MERGED, in));
  at[EV_MERGED] = 9000;
  TEST_ASSERT_EQUAL_STRING("1", text(REACT + EV_MERGED, in));
  in.now = 12001;
  TEST_ASSERT_EQUAL_STRING("0", text(REACT + EV_MERGED, in));
}

void test_tweens() {
  Tween t;
  t.ms = 1000;
  float f;
  TEST_ASSERT_TRUE(progress(t, 2250, 0, 0, f));
  TEST_ASSERT_EQUAL_FLOAT(0.25f, f);
  t.loop = Loop::PINGPONG;
  TEST_ASSERT_TRUE(progress(t, 1750, 0, 0, f));
  TEST_ASSERT_EQUAL_FLOAT(0.25f, f);
  t.loop = Loop::ONCE;
  TEST_ASSERT_TRUE(progress(t, 5000, 0, 0, f));
  TEST_ASSERT_EQUAL_FLOAT(1, f);
  t.loop = Loop::REPEAT;
  t.phase = 100;
  TEST_ASSERT_TRUE(progress(t, 1300, 0, 2, f));  // copy 2 runs 200 ms behind
  TEST_ASSERT_EQUAL_FLOAT(0.1f, f);
  // on an event: nothing before it, `times` cycles after, then nothing
  Tween j;
  j.on = EV_MERGED;
  j.ms = 280;
  j.times = 3;
  j.loop = Loop::PINGPONG;
  TEST_ASSERT_FALSE(progress(j, 5000, 0, 0, f));
  TEST_ASSERT_TRUE(progress(j, 5140, 5000, 0, f));
  TEST_ASSERT_EQUAL_FLOAT(0.5f, f);
  TEST_ASSERT_TRUE(progress(j, 5000 + 280 * 6 - 1, 5000, 0, f));
  TEST_ASSERT_FALSE(progress(j, 5000 + 280 * 6, 5000, 0, f));
}

void test_easing() {
  TEST_ASSERT_EQUAL_FLOAT(0, ease(Ease::SINE, 0));
  TEST_ASSERT_EQUAL_FLOAT(1, ease(Ease::SINE, 1));
  TEST_ASSERT_EQUAL_FLOAT(0.5f, ease(Ease::INOUT, 0.5f));
  TEST_ASSERT_EQUAL_FLOAT(0.25f, ease(Ease::IN, 0.5f));
  TEST_ASSERT_EQUAL_FLOAT(0.75f, ease(Ease::OUT, 0.5f));
  TEST_ASSERT_TRUE(ease(Ease::BACK, 0.7f) > 1);  // overshoots
  TEST_ASSERT_EQUAL_FLOAT(1, ease(Ease::BACK, 1));
}

void test_colours() {
  uint16_t c;
  TEST_ASSERT_TRUE(parseHex("#ffffff", c));
  TEST_ASSERT_EQUAL_HEX16(0xFFFF, c);
  TEST_ASSERT_TRUE(parseHex("#6CB8EF", c));
  TEST_ASSERT_EQUAL_HEX16(rgb565(0x6c, 0xb8, 0xef), c);
  TEST_ASSERT_FALSE(parseHex("#fff", c));
  TEST_ASSERT_FALSE(parseHex("#gggggg", c));
  TEST_ASSERT_FALSE(parseHex("sky", c));
  TEST_ASSERT_EQUAL_HEX16(0x0000, blend(0x0000, 0xFFFF, 0));
  TEST_ASSERT_EQUAL_HEX16(0xFFFF, blend(0x0000, 0xFFFF, 1));
}

void test_frame_tiers() {
  TEST_ASSERT_EQUAL(40, frameMs(LIVELY, false, false, 0));
  TEST_ASSERT_EQUAL(66, frameMs(LIVELY, false, true, 1000));
  TEST_ASSERT_EQUAL(150, frameMs(LIVELY, false, true, 30000));
  TEST_ASSERT_EQUAL(100, frameMs(CALM, false, false, 0));
  TEST_ASSERT_EQUAL(500, frameMs(CALM, false, true, 30000));
  TEST_ASSERT_EQUAL(0, frameMs(STATIC, false, false, 0));
  TEST_ASSERT_EQUAL(200, frameMs(STATIC, true, false, 0));
  TEST_ASSERT_EQUAL(1000, frameMs(STATIC, true, true, 0));
  TEST_ASSERT_EQUAL(0, frameMs(FROZEN, true, false, 0));
}

void test_guard_rail() {
  Guard g;
  uint32_t t = 1000;
  for (; t < 2900; t += 66) TEST_ASSERT_FALSE(g.feed(45000, t));  // slow, but not yet for 2 s
  TEST_ASSERT_TRUE(g.feed(45000, 3100));
  TEST_ASSERT_FALSE(g.feed(45000, 3166));  // the next tier gets its own 2 s
  // a fast frame in between starts over
  Guard h;
  TEST_ASSERT_FALSE(h.feed(45000, 0));
  TEST_ASSERT_FALSE(h.feed(45000, 1900));
  TEST_ASSERT_FALSE(h.feed(10000, 1950));
  TEST_ASSERT_FALSE(h.feed(45000, 2100));
  TEST_ASSERT_FALSE(h.feed(45000, 4000));
  TEST_ASSERT_TRUE(h.feed(45000, 4100));
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_binding_names);
  RUN_TEST(test_time_twelve_and_twenty_four);
  RUN_TEST(test_unknown_time_is_empty_and_false);
  RUN_TEST(test_date_parts_from_the_clock);
  RUN_TEST(test_world_and_pet);
  RUN_TEST(test_templates);
  RUN_TEST(test_expand_never_overflows);
  RUN_TEST(test_template_uses);
  RUN_TEST(test_numbers);
  RUN_TEST(test_conditions);
  RUN_TEST(test_condition_depth_is_bounded);
  RUN_TEST(test_reactions);
  RUN_TEST(test_tweens);
  RUN_TEST(test_easing);
  RUN_TEST(test_colours);
  RUN_TEST(test_frame_tiers);
  RUN_TEST(test_guard_rail);
  return UNITY_END();
}

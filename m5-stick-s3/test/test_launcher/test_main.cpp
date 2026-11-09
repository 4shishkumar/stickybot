// The Launcher's pure logic (src/logic/launcher.*): the tiles, the dashboard's order string,
// the grid and its pages, the quick settings' steps.
#include <unity.h>
#include <string.h>
#include "logic/launcher.h"

using namespace launcher;

void setUp() {}
void tearDown() {}

static const char* lettersOf(const Layout& l) {
  static char out[TILE_COUNT + 1];
  letters(l, out, sizeof out);
  return out;
}

void test_defaults_are_all_eight_in_order() {
  Layout l = defaults();
  TEST_ASSERT_EQUAL(8, l.n);
  TEST_ASSERT_EQUAL_STRING("grapmfqs", lettersOf(l));
  char buf[80];
  ids(l, buf, sizeof buf);
  TEST_ASSERT_EQUAL_STRING("games,remote,air,presenter,media,faces,quick,settings", buf);
}

void test_names_and_letters() {
  Tile t;
  TEST_ASSERT_TRUE(fromName("presenter", 9, t));
  TEST_ASSERT_EQUAL((int)Tile::PRESENTER, (int)t);
  TEST_ASSERT_TRUE(fromName("q", 1, t));
  TEST_ASSERT_EQUAL((int)Tile::QUICK, (int)t);
  TEST_ASSERT_FALSE(fromName("present", 7, t));
  TEST_ASSERT_FALSE(fromName("x", 1, t));
  TEST_ASSERT_FALSE(fromName("", 0, t));
  for (int i = 0; i < TILE_COUNT; i++)  // letters are unique
    for (int j = i + 1; j < TILE_COUNT; j++) TEST_ASSERT_NOT_EQUAL(info((Tile)i).letter, info((Tile)j).letter);
}

void test_parse_orders_and_hides() {
  Layout l = defaults();
  TEST_ASSERT_EQUAL((int)ParseError::NONE, (int)parse("faces, games ,settings,quick,air", l));
  TEST_ASSERT_EQUAL_STRING("fgsqa", lettersOf(l));
  TEST_ASSERT_EQUAL(-1, l.indexOf(Tile::REMOTE));
  TEST_ASSERT_EQUAL(2, l.indexOf(Tile::SETTINGS));
}

void test_parse_appends_the_fixed_tiles() {
  Layout l;
  TEST_ASSERT_EQUAL((int)ParseError::NONE, (int)parse("media,games", l));
  TEST_ASSERT_EQUAL_STRING("mgqs", lettersOf(l));
  TEST_ASSERT_EQUAL((int)ParseError::NONE, (int)parse("settings,g", l));  // letters work too
  TEST_ASSERT_EQUAL_STRING("sgq", lettersOf(l));
  TEST_ASSERT_EQUAL((int)ParseError::NONE, (int)parse("quick", l));
  TEST_ASSERT_EQUAL_STRING("qs", lettersOf(l));
}

void test_parse_errors_leave_the_layout_alone() {
  Layout l = defaults();
  char bad[16];
  TEST_ASSERT_EQUAL((int)ParseError::UNKNOWN, (int)parse("games,toaster", l, bad, sizeof bad));
  TEST_ASSERT_EQUAL_STRING("toaster", bad);
  TEST_ASSERT_EQUAL((int)ParseError::DUPLICATE, (int)parse("games,air,g", l, bad, sizeof bad));
  TEST_ASSERT_EQUAL_STRING("g", bad);
  TEST_ASSERT_EQUAL((int)ParseError::EMPTY, (int)parse("", l));
  TEST_ASSERT_EQUAL((int)ParseError::EMPTY, (int)parse(" , ,", l));
  TEST_ASSERT_EQUAL((int)ParseError::EMPTY, (int)parse(nullptr, l));
  TEST_ASSERT_TRUE(l == defaults());
  TEST_ASSERT_EQUAL_STRING("unknown tile", errorText(ParseError::UNKNOWN));
}

void test_letters_round_trip() {
  Layout l, back;
  TEST_ASSERT_EQUAL((int)ParseError::NONE, (int)parse("air,presenter,quick,games,settings", l));
  TEST_ASSERT_EQUAL((int)ParseError::NONE, (int)parseLetters(lettersOf(l), back));
  TEST_ASSERT_TRUE(l == back);
  TEST_ASSERT_EQUAL((int)ParseError::NONE, (int)parseLetters("ma", back));
  TEST_ASSERT_EQUAL_STRING("maqs", lettersOf(back));
  TEST_ASSERT_EQUAL((int)ParseError::UNKNOWN, (int)parseLetters("gz", back));
  TEST_ASSERT_EQUAL((int)ParseError::DUPLICATE, (int)parseLetters("gg", back));
  TEST_ASSERT_EQUAL((int)ParseError::EMPTY, (int)parseLetters("", back));
}

void test_ids_respects_the_buffer() {
  char small[12];
  ids(defaults(), small, sizeof small);
  TEST_ASSERT_EQUAL_STRING("games", small);  // "games,remote" doesn't fit in 11 + 1
  char tiny[3];
  letters(defaults(), tiny, sizeof tiny);
  TEST_ASSERT_EQUAL_STRING("gr", tiny);
}

void test_grid_cells_and_pages() {
  Grid p = gridFor(false), w = gridFor(true);
  TEST_ASSERT_EQUAL(2, p.cols);
  TEST_ASSERT_EQUAL(3, p.rows);
  TEST_ASSERT_EQUAL(3, w.cols);
  TEST_ASSERT_EQUAL(2, w.rows);
  TEST_ASSERT_EQUAL(6, p.perPage());
  TEST_ASSERT_EQUAL(6, w.perPage());
  Cell c = cellOf(3, p);
  TEST_ASSERT_EQUAL(0, c.page);
  TEST_ASSERT_EQUAL(1, c.col);
  TEST_ASSERT_EQUAL(1, c.row);
  c = cellOf(3, w);
  TEST_ASSERT_EQUAL(0, c.col);
  TEST_ASSERT_EQUAL(1, c.row);
  c = cellOf(7, p);
  TEST_ASSERT_EQUAL(1, c.page);
  TEST_ASSERT_EQUAL(1, c.col);
  TEST_ASSERT_EQUAL(0, c.row);
  TEST_ASSERT_EQUAL(2, pages(8, 6));
  TEST_ASSERT_EQUAL(1, pages(6, 6));
  TEST_ASSERT_EQUAL(1, pages(2, 6));
  TEST_ASSERT_EQUAL(1, pages(0, 6));
}

void test_next_and_prev_wrap() {
  TEST_ASSERT_EQUAL(1, next(0, 8));
  TEST_ASSERT_EQUAL(0, next(7, 8));
  TEST_ASSERT_EQUAL(7, prev(0, 8));
  TEST_ASSERT_EQUAL(0, next(0, 0));
}

void test_turn_page_keeps_the_place() {
  TEST_ASSERT_EQUAL(6, turnPage(0, 8, 6, +1));
  TEST_ASSERT_EQUAL(7, turnPage(4, 8, 6, +1));  // page 2 is shorter: its last tile
  TEST_ASSERT_EQUAL(1, turnPage(7, 8, 6, +1));  // round to page 1
  TEST_ASSERT_EQUAL(0, turnPage(6, 8, 6, -1));
  TEST_ASSERT_EQUAL(7, turnPage(1, 8, 6, -1));  // back round from page 1: the last page
  TEST_ASSERT_EQUAL(3, turnPage(3, 5, 6, +1));  // one page: stays
  TEST_ASSERT_EQUAL(13, turnPage(1, 14, 6, -1));  // 3 pages: page 3 slot 1 = 13
}

void test_start_at_the_last_opened() {
  Layout l = defaults();
  TEST_ASSERT_EQUAL(5, startAt(l, Tile::FACES));
  parse("games,quick", l);
  TEST_ASSERT_EQUAL(0, startAt(l, Tile::FACES));  // hidden now
  TEST_ASSERT_EQUAL(1, startAt(l, Tile::QUICK));
}

void test_quick_steps() {
  TEST_ASSERT_EQUAL(50, nextBrightness(25));
  TEST_ASSERT_EQUAL(100, nextBrightness(75));
  TEST_ASSERT_EQUAL(25, nextBrightness(100));
  TEST_ASSERT_EQUAL(25, nextBrightness(10));  // from the dashboard's 10-100: the next step up
  TEST_ASSERT_EQUAL(75, nextBrightness(60));
  TEST_ASSERT_EQUAL(128, nextVolume(64));
  TEST_ASSERT_EQUAL(255, nextVolume(192));
  TEST_ASSERT_EQUAL(64, nextVolume(255));
  TEST_ASSERT_EQUAL(64, nextVolume(10));
  TEST_ASSERT_EQUAL(128, nextVolume(100));
  TEST_ASSERT_EQUAL(100, volumePct(255));
  TEST_ASSERT_EQUAL(0, volumePct(0));
  TEST_ASSERT_EQUAL(50, volumePct(128));
}

void test_gestures_toggle_all_three() {
  bool lift = true, face = false, shake = false;
  TEST_ASSERT_TRUE(gesturesOn(lift, face, shake));
  toggleGestures(lift, face, shake);
  TEST_ASSERT_FALSE(lift || face || shake);
  toggleGestures(lift, face, shake);
  TEST_ASSERT_TRUE(lift && face && shake);
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_defaults_are_all_eight_in_order);
  RUN_TEST(test_names_and_letters);
  RUN_TEST(test_parse_orders_and_hides);
  RUN_TEST(test_parse_appends_the_fixed_tiles);
  RUN_TEST(test_parse_errors_leave_the_layout_alone);
  RUN_TEST(test_letters_round_trip);
  RUN_TEST(test_ids_respects_the_buffer);
  RUN_TEST(test_grid_cells_and_pages);
  RUN_TEST(test_next_and_prev_wrap);
  RUN_TEST(test_turn_page_keeps_the_place);
  RUN_TEST(test_start_at_the_last_opened);
  RUN_TEST(test_quick_steps);
  RUN_TEST(test_gestures_toggle_all_three);
  return UNITY_END();
}

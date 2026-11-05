// The sky over StickyWorld (and the light in the Aquarium) by time of day and weather.
// Pure C++ (native-testable). Times are minutes since local midnight (0-1439).
#pragma once
#include <stdint.h>

namespace sky {

struct Rgb {
  uint8_t r, g, b;
};

enum class Phase : uint8_t { NIGHT, DAWN, DAY, DUSK };

// Night until 5:30, dawn to 7:30, day to 17:30, dusk to 19:30, then night.
Phase phase(int minute);

struct Colors {
  Rgb   top, bottom;  // the gradient, top of the sky to the horizon
  float light;        // 0 deep night .. 1 full day
};
// Keyframed and blended; overcast (clouds, rain, snow, fog, storm) greys it out.
Colors colors(int minute, bool overcast);

// Where the sun / moon is on its arc across the sky: t 0 (rising, left) .. 1 (setting,
// right). false while it's below the horizon. Sun 6:00-19:00, moon 18:30-6:30.
bool sunAt(int minute, float& t);
bool moonAt(int minute, float& t);

// Weather codes from the agent (weather.py): clear | partly | cloudy | fog | rain | snow | storm
bool overcast(const char* icon);
int  clouds(const char* icon);      // how many clouds to draw (0-5)

Rgb mix(Rgb a, Rgb b, float t);     // t 0 -> a, 1 -> b

}  // namespace sky

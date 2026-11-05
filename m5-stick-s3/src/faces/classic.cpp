// Classic: Home as it was before faces (the clock, the weather and the agent's card, with
// the eyes peeking over it now and then). The smallest face: a template for new ones.
#include "faces.h"
#include "../ui_draw.h"

namespace faces {

namespace {

class Classic : public Face {
 public:
  const char* id() const override { return "classic"; }
  const char* title() const override { return "Classic"; }
  const char* about() const override { return "The big clock, the weather and a card from the Mac agent"; }
  void draw(const ui::Dash& d, bool, uint32_t) override { ui::drawClassicHome(d); }
  // frameMs 0: main.cpp redraws it while the card fades (ui::homeAnimating)
};

}  // namespace

Face& classic() {
  static Classic f;
  return f;
}

}  // namespace faces

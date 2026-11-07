# face_shots: the Home faces drawn on the Mac with the stick's own code (ui.cpp, src/faces/,
# LovyanGFX into a sprite), saved as PNGs. Needs the libraries PlatformIO downloads: run
# `pio run -e sticks3` (or `make build` one level up) once first.
#
#   make -C tools/face_shots            every face's demo, a picture per step -> out/
#   make -C tools/face_shots FACE=<id>  one face's demo
#   make -C tools/face_shots previews   the dashboard's face pictures (mac-agent static/faces/)
#   make -C tools/face_shots pkg PKG=<dir> [PREVIEWS=1]
#                                       a face package (docs/faces/FORMAT.md): validated and
#                                       compiled (tools/faces/validate.py), drawn in several
#                                       states -> out/pkg/<id>-*.png + <id>-sheet.png;
#                                       PREVIEWS=1 also writes <dir>/previews/*.png
SRC  := ../../src
LIBS := ../../.pio/libdeps/sticks3
GFX  := $(LIBS)/M5GFX/src
JSON := $(LIBS)/ArduinoJson/src
B    := build
FACE ?= all
PYTHON ?= python3
PKG  ?= ../../../faces/examples/textclock

CXXFLAGS := -std=gnu++17 -O1 -w -Imock -I$(SRC) -I$(GFX) -I$(JSON)
CFLAGS   := -O1 -w -Imock -I$(GFX)

APP  := shots.cpp $(SRC)/ui.cpp $(SRC)/fonts_vlw.cpp $(wildcard $(SRC)/faces/*.cpp) $(SRC)/faces/pkg/engine.cpp $(wildcard $(SRC)/logic/*.cpp)
LGFX := $(GFX)/lgfx/v1/LGFXBase.cpp $(GFX)/lgfx/v1/lgfx_v1.cpp $(GFX)/lgfx/v1/lgfx_fonts.cpp \
        $(GFX)/lgfx/v1/lgfx_v1_panel.cpp fbbase.cpp
CSRC := $(wildcard $(GFX)/lgfx/utility/*.c) fontstubs.c
OBJ  := $(addprefix $(B)/,$(notdir $(APP:.cpp=.o) $(LGFX:.cpp=.o) $(CSRC:.c=.o)))
VPATH := $(SRC) $(SRC)/faces $(SRC)/faces/pkg $(SRC)/logic $(GFX)/lgfx/v1 $(GFX)/lgfx/utility .

.PHONY: demo previews pkg clean
demo: $(B)/shots
	cd $(B)/.. && $(B)/shots demo out $(FACE)
previews: $(B)/shots
	$(B)/shots previews ../../../mac-agent/stick_agent/static/faces

pkg: $(B)/shots
	$(PYTHON) ../../../tools/faces/validate.py --no-previews --compile $(B)/sfb $(PKG)
	@mkdir -p out
	@id=$$($(PYTHON) -c "import json,sys; print(json.load(open(sys.argv[1]))['id'])" $(PKG)/manifest.json) && \
	  $(B)/shots pkg $(B)/sfb/$$id.sfb out/pkg && \
	  if [ -n "$(PREVIEWS)" ]; then $(B)/shots pkgpreview $(B)/sfb/$$id.sfb $(PKG)/previews; fi

$(B)/shots: $(OBJ)
	$(CXX) -o $@ $^ -lz
# a header changed (ui::Dash, an engine struct): rebuild what includes it, or the objects disagree
HDRS := $(wildcard $(SRC)/*.h $(SRC)/faces/*.h $(SRC)/faces/pkg/*.h $(SRC)/logic/*.h)
$(addprefix $(B)/,$(notdir $(APP:.cpp=.o))): $(HDRS)
$(B)/%.o: %.cpp | $(B)
	$(CXX) $(CXXFLAGS) -c $< -o $@
$(B)/%.o: %.c | $(B)
	$(CC) $(CFLAGS) -c $< -o $@
$(B):
	mkdir -p $(B)
clean:
	rm -rf $(B) out

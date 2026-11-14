# M5StickS3 firmware — common tasks (PlatformIO in ~/.platformio/penv).
#   make test     unit tests on the Mac (src/logic/, no stick needed)
#   make build    build the stick firmware
#   make flash    build + flash over USB (first stick on USB)
#   make fonts    regenerate src/fonts_vlw.* (needs Pillow)
#   make sounds   regenerate src/sounds_clips.* (needs numpy, soundfile, soxr)
#   make release  build + the flash parts + manifest in ~/.stick-agent/firmware/local (the agent flashes it)
PIO    ?= $(HOME)/.platformio/penv/bin/pio
PYTHON ?= $(HOME)/.stick-agent/venv/bin/python
PORT   ?= $(firstword $(wildcard /dev/cu.usbmodem14C*) $(wildcard /dev/cu.usbmodem*))

.PHONY: test build flash build-plus2 flash-plus2 fonts sounds release clean

test:
	$(PIO) test -e native

build:
	$(PIO) run -e sticks3

flash:
	$(PIO) run -e sticks3 -t upload $(if $(PORT),--upload-port $(PORT))

# M5StickC Plus2 (src/board.h): make flash-plus2 PLUS2_PORT=/dev/cu.usbserial-...
PLUS2_PORT ?= $(firstword $(wildcard /dev/cu.usbserial-*))
build-plus2:
	$(PIO) run -e stickc_plus2

flash-plus2:
	$(PIO) run -e stickc_plus2 -t upload $(if $(PLUS2_PORT),--upload-port $(PLUS2_PORT))

fonts:
	$(PYTHON) tools/make_fonts.py

sounds:
	$(PYTHON) tools/make_sounds.py

release: build
	python3 tools/make_release.py

clean:
	$(PIO) run -t clean -e sticks3
	rm -rf .pio/build/native

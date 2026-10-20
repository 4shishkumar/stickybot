# PlatformIO pre-build script: embeds web/settings.html into src/web_page.h
# so the firmware can serve it. Edit the HTML, then build as usual.
Import("env")  # noqa: F821 (provided by PlatformIO)
import os

root = env.subst("$PROJECT_DIR")  # noqa: F821
src = os.path.join(root, "web", "settings.html")
dst = os.path.join(root, "src", "web_page.h")

html = open(src, encoding="utf-8").read()
assert ')HTML"' not in html
out = ("// Generated from web/settings.html by tools/embed_page.py — don't edit.\n"
       "#pragma once\n#include <Arduino.h>\n\n"
       'static const char WEB_PAGE[] PROGMEM = R"HTML(' + html + ')HTML";\n')

if not os.path.exists(dst) or open(dst, encoding="utf-8").read() != out:
    open(dst, "w", encoding="utf-8").write(out)
    print("Embedded web/settings.html -> src/web_page.h")

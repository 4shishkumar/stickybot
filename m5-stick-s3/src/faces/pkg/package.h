// Face packages in the faces registry (faces.cpp): a PackageFace for each installed face
// (store.cpp), after the built-in ones, so they're in the pickers, "@face id=" and the demo
// like any face. Only the face on screen is loaded (one Engine, its arena in PSRAM); a face
// that doesn't load draws Classic and says why ("Faces: <id> invalid: <why>", @faces "err").
//
// The crash guard (FORMAT.md section 16): if the stick restarts twice in a row from a crash or a
// watchdog while a package face is the Home face, it boots with Classic and marks that face
// as broken until it's installed again. A minute of normal running clears the count.
#pragma once
#include "../faces.h"

namespace faces {
namespace pkg {

void     begin();                 // mounts the storage, lists the faces, runs the crash guard
void     loop();                  // a minute in: the crash guard's count goes back to 0
int      count();                 // installed package faces
Face&    at(int i);               // 0 .. count() - 1
bool     isPackage(const Face& f);
void     rescan();                // the folder changed (an install, a removal): indices change
void     installed(const char* id);  // a new / updated face arrived: forget a crash mark, reload it
// Over Home, on top of everything: the install toast ("Installing Cat Nap 60 %")
void     drawOverlay(bool wide);
uint32_t overlayFrameMs();        // how often Home redraws for it (0: nothing showing)

// @faces (FORMAT.md section 19): one page of installed faces; @state's "pkgFaces"
String   listJson(int page);
String   stateJson();
// Removes a face ("all": every one). The Home face falls back to Classic if it was one of
// them. How many were removed (0: not installed).
int      remove(const String& id);

}  // namespace pkg
}  // namespace faces

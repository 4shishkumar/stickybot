// Prompts on the stick: Claude Code approvals and (firmware 1.7.2) questions from a
// Claude session (the agent's stick tools, ask()). Both arrive as {"src":"cli","prompt":{…}};
// a question has "kind":"ask", the question in "q" and button labels in "a" / "b".
#pragma once
#include <string>

namespace prompt {

enum class Input { A, B, SHAKE };

// What to send back in {"cmd":"permission","decision":…}: A = "once", B = "deny";
// a shake denies an approval but only dismisses a question ("dismiss").
const char* decision(bool ask, Input in);

// A button's text: "A  Staging", or "A  yes" without a label; the label is cut to
// maxChars characters (UTF-8 aware, never mid-character).
std::string button(char key, const std::string& label, const char* fallback, size_t maxChars);

}  // namespace prompt

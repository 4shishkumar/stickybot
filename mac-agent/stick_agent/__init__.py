"""StickyBot: a voice assistant for your Mac, driven by an M5StickS3.

Hold B on the stick and speak. The agent turns speech into text on the Mac
(Parakeet, local), runs a quick command if one matches ("PR 1234"), and
otherwise hands the request to Claude Code (your own login), with permission
prompts answered on the stick.
"""
__version__ = "1.8.6"

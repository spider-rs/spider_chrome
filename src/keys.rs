//! Code based on [rust-headless-chrome](https://github.com/atroche/rust-headless-chrome/blob/master/src/browser/tab/keys.rs)

/// Represents a key on a keyboard
#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub struct KeyDefinition {
    /// The key value (e.g., "a", "Enter").
    pub key: &'static str,
    /// The platform-specific key code.
    pub key_code: i64,
    /// The physical key location (e.g., "KeyA", "Digit1").
    pub code: &'static str,
    /// The printable text, if applicable.
    pub text: Option<&'static str>,
}

//  Generated the following in node using Puppeteer:
//  keys = require("./lib/USKeyboardLayout.js")
//  toStruct = (kD) => `KeyDefinition { key: "${kD.key}", key_code:
// "${kD.keyCode}", code: "${kD.code}", text: kD.text }`  output = Object.
// values(keys).map(toStruct).join(",\n")  const fs = require('fs');
//  fs.writeFile("/tmp/blah1", output)

pub const USKEYBOARD_LAYOUT: [KeyDefinition; 244] = [
    KeyDefinition {
        key: "0",
        key_code: 48,
        code: "Digit0",
        text: None,
    },
    KeyDefinition {
        key: "1",
        key_code: 49,
        code: "Digit1",
        text: None,
    },
    KeyDefinition {
        key: "2",
        key_code: 50,
        code: "Digit2",
        text: None,
    },
    KeyDefinition {
        key: "3",
        key_code: 51,
        code: "Digit3",
        text: None,
    },
    KeyDefinition {
        key: "4",
        key_code: 52,
        code: "Digit4",
        text: None,
    },
    KeyDefinition {
        key: "5",
        key_code: 53,
        code: "Digit5",
        text: None,
    },
    KeyDefinition {
        key: "6",
        key_code: 54,
        code: "Digit6",
        text: None,
    },
    KeyDefinition {
        key: "7",
        key_code: 55,
        code: "Digit7",
        text: None,
    },
    KeyDefinition {
        key: "8",
        key_code: 56,
        code: "Digit8",
        text: None,
    },
    KeyDefinition {
        key: "9",
        key_code: 57,
        code: "Digit9",
        text: None,
    },
    KeyDefinition {
        key: "Power",
        key_code: 0,
        code: "Power",
        text: None,
    },
    KeyDefinition {
        key: "Eject",
        key_code: 0,
        code: "Eject",
        text: None,
    },
    KeyDefinition {
        key: "Cancel",
        key_code: 3,
        code: "Abort",
        text: None,
    },
    KeyDefinition {
        key: "Help",
        key_code: 6,
        code: "Help",
        text: None,
    },
    KeyDefinition {
        key: "Backspace",
        key_code: 8,
        code: "Backspace",
        text: None,
    },
    KeyDefinition {
        key: "Tab",
        key_code: 9,
        code: "Tab",
        text: None,
    },
    KeyDefinition {
        key: "Clear",
        key_code: 12,
        code: "Numpad5",
        text: None,
    },
    KeyDefinition {
        key: "Enter",
        key_code: 13,
        code: "Enter",
        text: Some("\r"),
    },
    KeyDefinition {
        key: "Shift",
        key_code: 16,
        code: "ShiftLeft",
        text: None,
    },
    KeyDefinition {
        key: "Shift",
        key_code: 16,
        code: "ShiftRight",
        text: None,
    },
    KeyDefinition {
        key: "Control",
        key_code: 17,
        code: "ControlLeft",
        text: None,
    },
    KeyDefinition {
        key: "Control",
        key_code: 17,
        code: "ControlRight",
        text: None,
    },
    KeyDefinition {
        key: "Alt",
        key_code: 18,
        code: "AltLeft",
        text: None,
    },
    KeyDefinition {
        key: "Alt",
        key_code: 18,
        code: "AltRight",
        text: None,
    },
    KeyDefinition {
        key: "Pause",
        key_code: 19,
        code: "Pause",
        text: None,
    },
    KeyDefinition {
        key: "CapsLock",
        key_code: 20,
        code: "CapsLock",
        text: None,
    },
    KeyDefinition {
        key: "Escape",
        key_code: 27,
        code: "Escape",
        text: None,
    },
    KeyDefinition {
        key: "Convert",
        key_code: 28,
        code: "Convert",
        text: None,
    },
    KeyDefinition {
        key: "NonConvert",
        key_code: 29,
        code: "NonConvert",
        text: None,
    },
    KeyDefinition {
        key: " ",
        key_code: 32,
        code: "Space",
        text: None,
    },
    KeyDefinition {
        key: "PageUp",
        key_code: 33,
        code: "Numpad9",
        text: None,
    },
    KeyDefinition {
        key: "PageUp",
        key_code: 33,
        code: "PageUp",
        text: None,
    },
    KeyDefinition {
        key: "PageDown",
        key_code: 34,
        code: "Numpad3",
        text: None,
    },
    KeyDefinition {
        key: "PageDown",
        key_code: 34,
        code: "PageDown",
        text: None,
    },
    KeyDefinition {
        key: "End",
        key_code: 35,
        code: "End",
        text: None,
    },
    KeyDefinition {
        key: "End",
        key_code: 35,
        code: "Numpad1",
        text: None,
    },
    KeyDefinition {
        key: "Home",
        key_code: 36,
        code: "Home",
        text: None,
    },
    KeyDefinition {
        key: "Home",
        key_code: 36,
        code: "Numpad7",
        text: None,
    },
    KeyDefinition {
        key: "ArrowLeft",
        key_code: 37,
        code: "ArrowLeft",
        text: None,
    },
    KeyDefinition {
        key: "ArrowLeft",
        key_code: 37,
        code: "Numpad4",
        text: None,
    },
    KeyDefinition {
        key: "ArrowUp",
        key_code: 38,
        code: "Numpad8",
        text: None,
    },
    KeyDefinition {
        key: "ArrowUp",
        key_code: 38,
        code: "ArrowUp",
        text: None,
    },
    KeyDefinition {
        key: "ArrowRight",
        key_code: 39,
        code: "ArrowRight",
        text: None,
    },
    KeyDefinition {
        key: "ArrowRight",
        key_code: 39,
        code: "Numpad6",
        text: None,
    },
    KeyDefinition {
        key: "ArrowDown",
        key_code: 40,
        code: "Numpad2",
        text: None,
    },
    KeyDefinition {
        key: "ArrowDown",
        key_code: 40,
        code: "ArrowDown",
        text: None,
    },
    KeyDefinition {
        key: "Select",
        key_code: 41,
        code: "Select",
        text: None,
    },
    KeyDefinition {
        key: "Execute",
        key_code: 43,
        code: "Open",
        text: None,
    },
    KeyDefinition {
        key: "PrintScreen",
        key_code: 44,
        code: "PrintScreen",
        text: None,
    },
    KeyDefinition {
        key: "Insert",
        key_code: 45,
        code: "Insert",
        text: None,
    },
    KeyDefinition {
        key: "Insert",
        key_code: 45,
        code: "Numpad0",
        text: None,
    },
    KeyDefinition {
        key: "Delete",
        key_code: 46,
        code: "Delete",
        text: None,
    },
    KeyDefinition {
        key: " ",
        key_code: 46,
        code: "NumpadDecimal",
        text: None,
    },
    KeyDefinition {
        key: "0",
        key_code: 48,
        code: "Digit0",
        text: None,
    },
    KeyDefinition {
        key: "1",
        key_code: 49,
        code: "Digit1",
        text: None,
    },
    KeyDefinition {
        key: "2",
        key_code: 50,
        code: "Digit2",
        text: None,
    },
    KeyDefinition {
        key: "3",
        key_code: 51,
        code: "Digit3",
        text: None,
    },
    KeyDefinition {
        key: "4",
        key_code: 52,
        code: "Digit4",
        text: None,
    },
    KeyDefinition {
        key: "5",
        key_code: 53,
        code: "Digit5",
        text: None,
    },
    KeyDefinition {
        key: "6",
        key_code: 54,
        code: "Digit6",
        text: None,
    },
    KeyDefinition {
        key: "7",
        key_code: 55,
        code: "Digit7",
        text: None,
    },
    KeyDefinition {
        key: "8",
        key_code: 56,
        code: "Digit8",
        text: None,
    },
    KeyDefinition {
        key: "9",
        key_code: 57,
        code: "Digit9",
        text: None,
    },
    KeyDefinition {
        key: "a",
        key_code: 65,
        code: "KeyA",
        text: None,
    },
    KeyDefinition {
        key: "b",
        key_code: 66,
        code: "KeyB",
        text: None,
    },
    KeyDefinition {
        key: "c",
        key_code: 67,
        code: "KeyC",
        text: None,
    },
    KeyDefinition {
        key: "d",
        key_code: 68,
        code: "KeyD",
        text: None,
    },
    KeyDefinition {
        key: "e",
        key_code: 69,
        code: "KeyE",
        text: None,
    },
    KeyDefinition {
        key: "f",
        key_code: 70,
        code: "KeyF",
        text: None,
    },
    KeyDefinition {
        key: "g",
        key_code: 71,
        code: "KeyG",
        text: None,
    },
    KeyDefinition {
        key: "h",
        key_code: 72,
        code: "KeyH",
        text: None,
    },
    KeyDefinition {
        key: "i",
        key_code: 73,
        code: "KeyI",
        text: None,
    },
    KeyDefinition {
        key: "j",
        key_code: 74,
        code: "KeyJ",
        text: None,
    },
    KeyDefinition {
        key: "k",
        key_code: 75,
        code: "KeyK",
        text: None,
    },
    KeyDefinition {
        key: "l",
        key_code: 76,
        code: "KeyL",
        text: None,
    },
    KeyDefinition {
        key: "m",
        key_code: 77,
        code: "KeyM",
        text: None,
    },
    KeyDefinition {
        key: "n",
        key_code: 78,
        code: "KeyN",
        text: None,
    },
    KeyDefinition {
        key: "o",
        key_code: 79,
        code: "KeyO",
        text: None,
    },
    KeyDefinition {
        key: "p",
        key_code: 80,
        code: "KeyP",
        text: None,
    },
    KeyDefinition {
        key: "q",
        key_code: 81,
        code: "KeyQ",
        text: None,
    },
    KeyDefinition {
        key: "r",
        key_code: 82,
        code: "KeyR",
        text: None,
    },
    KeyDefinition {
        key: "s",
        key_code: 83,
        code: "KeyS",
        text: None,
    },
    KeyDefinition {
        key: "t",
        key_code: 84,
        code: "KeyT",
        text: None,
    },
    KeyDefinition {
        key: "u",
        key_code: 85,
        code: "KeyU",
        text: None,
    },
    KeyDefinition {
        key: "v",
        key_code: 86,
        code: "KeyV",
        text: None,
    },
    KeyDefinition {
        key: "w",
        key_code: 87,
        code: "KeyW",
        text: None,
    },
    KeyDefinition {
        key: "x",
        key_code: 88,
        code: "KeyX",
        text: None,
    },
    KeyDefinition {
        key: "y",
        key_code: 89,
        code: "KeyY",
        text: None,
    },
    KeyDefinition {
        key: "z",
        key_code: 90,
        code: "KeyZ",
        text: None,
    },
    KeyDefinition {
        key: "Meta",
        key_code: 91,
        code: "MetaLeft",
        text: None,
    },
    KeyDefinition {
        key: "Meta",
        key_code: 92,
        code: "MetaRight",
        text: None,
    },
    KeyDefinition {
        key: "ContextMenu",
        key_code: 93,
        code: "ContextMenu",
        text: None,
    },
    KeyDefinition {
        key: "*",
        key_code: 106,
        code: "NumpadMultiply",
        text: None,
    },
    KeyDefinition {
        key: "+",
        key_code: 107,
        code: "NumpadAdd",
        text: None,
    },
    KeyDefinition {
        key: "-",
        key_code: 109,
        code: "NumpadSubtract",
        text: None,
    },
    KeyDefinition {
        key: "/",
        key_code: 111,
        code: "NumpadDivide",
        text: None,
    },
    KeyDefinition {
        key: "F1",
        key_code: 112,
        code: "F1",
        text: None,
    },
    KeyDefinition {
        key: "F2",
        key_code: 113,
        code: "F2",
        text: None,
    },
    KeyDefinition {
        key: "F3",
        key_code: 114,
        code: "F3",
        text: None,
    },
    KeyDefinition {
        key: "F4",
        key_code: 115,
        code: "F4",
        text: None,
    },
    KeyDefinition {
        key: "F5",
        key_code: 116,
        code: "F5",
        text: None,
    },
    KeyDefinition {
        key: "F6",
        key_code: 117,
        code: "F6",
        text: None,
    },
    KeyDefinition {
        key: "F7",
        key_code: 118,
        code: "F7",
        text: None,
    },
    KeyDefinition {
        key: "F8",
        key_code: 119,
        code: "F8",
        text: None,
    },
    KeyDefinition {
        key: "F9",
        key_code: 120,
        code: "F9",
        text: None,
    },
    KeyDefinition {
        key: "F10",
        key_code: 121,
        code: "F10",
        text: None,
    },
    KeyDefinition {
        key: "F11",
        key_code: 122,
        code: "F11",
        text: None,
    },
    KeyDefinition {
        key: "F12",
        key_code: 123,
        code: "F12",
        text: None,
    },
    KeyDefinition {
        key: "F13",
        key_code: 124,
        code: "F13",
        text: None,
    },
    KeyDefinition {
        key: "F14",
        key_code: 125,
        code: "F14",
        text: None,
    },
    KeyDefinition {
        key: "F15",
        key_code: 126,
        code: "F15",
        text: None,
    },
    KeyDefinition {
        key: "F16",
        key_code: 127,
        code: "F16",
        text: None,
    },
    KeyDefinition {
        key: "F17",
        key_code: 128,
        code: "F17",
        text: None,
    },
    KeyDefinition {
        key: "F18",
        key_code: 129,
        code: "F18",
        text: None,
    },
    KeyDefinition {
        key: "F19",
        key_code: 130,
        code: "F19",
        text: None,
    },
    KeyDefinition {
        key: "F20",
        key_code: 131,
        code: "F20",
        text: None,
    },
    KeyDefinition {
        key: "F21",
        key_code: 132,
        code: "F21",
        text: None,
    },
    KeyDefinition {
        key: "F22",
        key_code: 133,
        code: "F22",
        text: None,
    },
    KeyDefinition {
        key: "F23",
        key_code: 134,
        code: "F23",
        text: None,
    },
    KeyDefinition {
        key: "F24",
        key_code: 135,
        code: "F24",
        text: None,
    },
    KeyDefinition {
        key: "NumLock",
        key_code: 144,
        code: "NumLock",
        text: None,
    },
    KeyDefinition {
        key: "ScrollLock",
        key_code: 145,
        code: "ScrollLock",
        text: None,
    },
    KeyDefinition {
        key: "AudioVolumeMute",
        key_code: 173,
        code: "AudioVolumeMute",
        text: None,
    },
    KeyDefinition {
        key: "AudioVolumeDown",
        key_code: 174,
        code: "AudioVolumeDown",
        text: None,
    },
    KeyDefinition {
        key: "AudioVolumeUp",
        key_code: 175,
        code: "AudioVolumeUp",
        text: None,
    },
    KeyDefinition {
        key: "MediaTrackNext",
        key_code: 176,
        code: "MediaTrackNext",
        text: None,
    },
    KeyDefinition {
        key: "MediaTrackPrevious",
        key_code: 177,
        code: "MediaTrackPrevious",
        text: None,
    },
    KeyDefinition {
        key: "MediaStop",
        key_code: 178,
        code: "MediaStop",
        text: None,
    },
    KeyDefinition {
        key: "MediaPlayPause",
        key_code: 179,
        code: "MediaPlayPause",
        text: None,
    },
    KeyDefinition {
        key: ";",
        key_code: 186,
        code: "Semicolon",
        text: None,
    },
    KeyDefinition {
        key: "=",
        key_code: 187,
        code: "Equal",
        text: None,
    },
    KeyDefinition {
        key: "=",
        key_code: 187,
        code: "NumpadEqual",
        text: None,
    },
    KeyDefinition {
        key: ",",
        key_code: 188,
        code: "Comma",
        text: None,
    },
    KeyDefinition {
        key: "-",
        key_code: 189,
        code: "Minus",
        text: None,
    },
    KeyDefinition {
        key: ".",
        key_code: 190,
        code: "Period",
        text: None,
    },
    KeyDefinition {
        key: "/",
        key_code: 191,
        code: "Slash",
        text: None,
    },
    KeyDefinition {
        key: "`",
        key_code: 192,
        code: "Backquote",
        text: None,
    },
    KeyDefinition {
        key: "[",
        key_code: 219,
        code: "BracketLeft",
        text: None,
    },
    KeyDefinition {
        key: "\\",
        key_code: 220,
        code: "Backslash",
        text: None,
    },
    KeyDefinition {
        key: "]",
        key_code: 221,
        code: "BracketRight",
        text: None,
    },
    KeyDefinition {
        key: "'",
        key_code: 222,
        code: "Quote",
        text: None,
    },
    KeyDefinition {
        key: "AltGraph",
        key_code: 225,
        code: "AltGraph",
        text: None,
    },
    KeyDefinition {
        key: "CrSel",
        key_code: 247,
        code: "Props",
        text: None,
    },
    KeyDefinition {
        key: "Cancel",
        key_code: 3,
        code: "Abort",
        text: None,
    },
    KeyDefinition {
        key: "Clear",
        key_code: 12,
        code: "Numpad5",
        text: None,
    },
    KeyDefinition {
        key: "Shift",
        key_code: 16,
        code: "ShiftLeft",
        text: None,
    },
    KeyDefinition {
        key: "Control",
        key_code: 17,
        code: "ControlLeft",
        text: None,
    },
    KeyDefinition {
        key: "Alt",
        key_code: 18,
        code: "AltLeft",
        text: None,
    },
    KeyDefinition {
        key: "Accept",
        key_code: 30,
        code: "undefined",
        text: None,
    },
    KeyDefinition {
        key: "ModeChange",
        key_code: 31,
        code: "undefined",
        text: None,
    },
    KeyDefinition {
        key: " ",
        key_code: 32,
        code: "Space",
        text: None,
    },
    KeyDefinition {
        key: "Print",
        key_code: 42,
        code: "undefined",
        text: None,
    },
    KeyDefinition {
        key: "Execute",
        key_code: 43,
        code: "Open",
        text: None,
    },
    KeyDefinition {
        key: " ",
        key_code: 46,
        code: "NumpadDecimal",
        text: None,
    },
    KeyDefinition {
        key: "a",
        key_code: 65,
        code: "KeyA",
        text: None,
    },
    KeyDefinition {
        key: "b",
        key_code: 66,
        code: "KeyB",
        text: None,
    },
    KeyDefinition {
        key: "c",
        key_code: 67,
        code: "KeyC",
        text: None,
    },
    KeyDefinition {
        key: "d",
        key_code: 68,
        code: "KeyD",
        text: None,
    },
    KeyDefinition {
        key: "e",
        key_code: 69,
        code: "KeyE",
        text: None,
    },
    KeyDefinition {
        key: "f",
        key_code: 70,
        code: "KeyF",
        text: None,
    },
    KeyDefinition {
        key: "g",
        key_code: 71,
        code: "KeyG",
        text: None,
    },
    KeyDefinition {
        key: "h",
        key_code: 72,
        code: "KeyH",
        text: None,
    },
    KeyDefinition {
        key: "i",
        key_code: 73,
        code: "KeyI",
        text: None,
    },
    KeyDefinition {
        key: "j",
        key_code: 74,
        code: "KeyJ",
        text: None,
    },
    KeyDefinition {
        key: "k",
        key_code: 75,
        code: "KeyK",
        text: None,
    },
    KeyDefinition {
        key: "l",
        key_code: 76,
        code: "KeyL",
        text: None,
    },
    KeyDefinition {
        key: "m",
        key_code: 77,
        code: "KeyM",
        text: None,
    },
    KeyDefinition {
        key: "n",
        key_code: 78,
        code: "KeyN",
        text: None,
    },
    KeyDefinition {
        key: "o",
        key_code: 79,
        code: "KeyO",
        text: None,
    },
    KeyDefinition {
        key: "p",
        key_code: 80,
        code: "KeyP",
        text: None,
    },
    KeyDefinition {
        key: "q",
        key_code: 81,
        code: "KeyQ",
        text: None,
    },
    KeyDefinition {
        key: "r",
        key_code: 82,
        code: "KeyR",
        text: None,
    },
    KeyDefinition {
        key: "s",
        key_code: 83,
        code: "KeyS",
        text: None,
    },
    KeyDefinition {
        key: "t",
        key_code: 84,
        code: "KeyT",
        text: None,
    },
    KeyDefinition {
        key: "u",
        key_code: 85,
        code: "KeyU",
        text: None,
    },
    KeyDefinition {
        key: "v",
        key_code: 86,
        code: "KeyV",
        text: None,
    },
    KeyDefinition {
        key: "w",
        key_code: 87,
        code: "KeyW",
        text: None,
    },
    KeyDefinition {
        key: "x",
        key_code: 88,
        code: "KeyX",
        text: None,
    },
    KeyDefinition {
        key: "y",
        key_code: 89,
        code: "KeyY",
        text: None,
    },
    KeyDefinition {
        key: "z",
        key_code: 90,
        code: "KeyZ",
        text: None,
    },
    KeyDefinition {
        key: "Meta",
        key_code: 91,
        code: "MetaLeft",
        text: None,
    },
    KeyDefinition {
        key: "*",
        key_code: 106,
        code: "NumpadMultiply",
        text: None,
    },
    KeyDefinition {
        key: "+",
        key_code: 107,
        code: "NumpadAdd",
        text: None,
    },
    KeyDefinition {
        key: "-",
        key_code: 109,
        code: "NumpadSubtract",
        text: None,
    },
    KeyDefinition {
        key: "/",
        key_code: 111,
        code: "NumpadDivide",
        text: None,
    },
    KeyDefinition {
        key: ";",
        key_code: 186,
        code: "Semicolon",
        text: None,
    },
    KeyDefinition {
        key: "=",
        key_code: 187,
        code: "Equal",
        text: None,
    },
    KeyDefinition {
        key: ",",
        key_code: 188,
        code: "Comma",
        text: None,
    },
    KeyDefinition {
        key: ".",
        key_code: 190,
        code: "Period",
        text: None,
    },
    KeyDefinition {
        key: "`",
        key_code: 192,
        code: "Backquote",
        text: None,
    },
    KeyDefinition {
        key: "[",
        key_code: 219,
        code: "BracketLeft",
        text: None,
    },
    KeyDefinition {
        key: "]",
        key_code: 221,
        code: "BracketRight",
        text: None,
    },
    KeyDefinition {
        key: "'",
        key_code: 222,
        code: "Quote",
        text: None,
    },
    KeyDefinition {
        key: "Attn",
        key_code: 246,
        code: "undefined",
        text: None,
    },
    KeyDefinition {
        key: "CrSel",
        key_code: 247,
        code: "Props",
        text: None,
    },
    KeyDefinition {
        key: "ExSel",
        key_code: 248,
        code: "undefined",
        text: None,
    },
    KeyDefinition {
        key: "EraseEof",
        key_code: 249,
        code: "undefined",
        text: None,
    },
    KeyDefinition {
        key: "Play",
        key_code: 250,
        code: "undefined",
        text: None,
    },
    KeyDefinition {
        key: "ZoomOut",
        key_code: 251,
        code: "undefined",
        text: None,
    },
    KeyDefinition {
        key: ")",
        key_code: 48,
        code: "Digit0",
        text: None,
    },
    KeyDefinition {
        key: "!",
        key_code: 49,
        code: "Digit1",
        text: None,
    },
    KeyDefinition {
        key: "@",
        key_code: 50,
        code: "Digit2",
        text: None,
    },
    KeyDefinition {
        key: "#",
        key_code: 51,
        code: "Digit3",
        text: None,
    },
    KeyDefinition {
        key: "$",
        key_code: 52,
        code: "Digit4",
        text: None,
    },
    KeyDefinition {
        key: "%",
        key_code: 53,
        code: "Digit5",
        text: None,
    },
    KeyDefinition {
        key: "^",
        key_code: 54,
        code: "Digit6",
        text: None,
    },
    KeyDefinition {
        key: "&",
        key_code: 55,
        code: "Digit7",
        text: None,
    },
    KeyDefinition {
        key: "(",
        key_code: 57,
        code: "Digit9",
        text: None,
    },
    KeyDefinition {
        key: "A",
        key_code: 65,
        code: "KeyA",
        text: None,
    },
    KeyDefinition {
        key: "B",
        key_code: 66,
        code: "KeyB",
        text: None,
    },
    KeyDefinition {
        key: "C",
        key_code: 67,
        code: "KeyC",
        text: None,
    },
    KeyDefinition {
        key: "D",
        key_code: 68,
        code: "KeyD",
        text: None,
    },
    KeyDefinition {
        key: "E",
        key_code: 69,
        code: "KeyE",
        text: None,
    },
    KeyDefinition {
        key: "F",
        key_code: 70,
        code: "KeyF",
        text: None,
    },
    KeyDefinition {
        key: "G",
        key_code: 71,
        code: "KeyG",
        text: None,
    },
    KeyDefinition {
        key: "H",
        key_code: 72,
        code: "KeyH",
        text: None,
    },
    KeyDefinition {
        key: "I",
        key_code: 73,
        code: "KeyI",
        text: None,
    },
    KeyDefinition {
        key: "J",
        key_code: 74,
        code: "KeyJ",
        text: None,
    },
    KeyDefinition {
        key: "K",
        key_code: 75,
        code: "KeyK",
        text: None,
    },
    KeyDefinition {
        key: "L",
        key_code: 76,
        code: "KeyL",
        text: None,
    },
    KeyDefinition {
        key: "M",
        key_code: 77,
        code: "KeyM",
        text: None,
    },
    KeyDefinition {
        key: "N",
        key_code: 78,
        code: "KeyN",
        text: None,
    },
    KeyDefinition {
        key: "O",
        key_code: 79,
        code: "KeyO",
        text: None,
    },
    KeyDefinition {
        key: "P",
        key_code: 80,
        code: "KeyP",
        text: None,
    },
    KeyDefinition {
        key: "Q",
        key_code: 81,
        code: "KeyQ",
        text: None,
    },
    KeyDefinition {
        key: "R",
        key_code: 82,
        code: "KeyR",
        text: None,
    },
    KeyDefinition {
        key: "S",
        key_code: 83,
        code: "KeyS",
        text: None,
    },
    KeyDefinition {
        key: "T",
        key_code: 84,
        code: "KeyT",
        text: None,
    },
    KeyDefinition {
        key: "U",
        key_code: 85,
        code: "KeyU",
        text: None,
    },
    KeyDefinition {
        key: "V",
        key_code: 86,
        code: "KeyV",
        text: None,
    },
    KeyDefinition {
        key: "W",
        key_code: 87,
        code: "KeyW",
        text: None,
    },
    KeyDefinition {
        key: "X",
        key_code: 88,
        code: "KeyX",
        text: None,
    },
    KeyDefinition {
        key: "Y",
        key_code: 89,
        code: "KeyY",
        text: None,
    },
    KeyDefinition {
        key: "Z",
        key_code: 90,
        code: "KeyZ",
        text: None,
    },
    KeyDefinition {
        key: ":",
        key_code: 186,
        code: "Semicolon",
        text: None,
    },
    KeyDefinition {
        key: "<",
        key_code: 188,
        code: "Comma",
        text: None,
    },
    KeyDefinition {
        key: "_",
        key_code: 189,
        code: "Minus",
        text: None,
    },
    KeyDefinition {
        key: ">",
        key_code: 190,
        code: "Period",
        text: None,
    },
    KeyDefinition {
        key: "?",
        key_code: 191,
        code: "Slash",
        text: None,
    },
    KeyDefinition {
        key: "~",
        key_code: 192,
        code: "Backquote",
        text: None,
    },
    KeyDefinition {
        key: "{",
        key_code: 219,
        code: "BracketLeft",
        text: None,
    },
    KeyDefinition {
        key: "|",
        key_code: 220,
        code: "Backslash",
        text: None,
    },
    KeyDefinition {
        key: "}",
        key_code: 221,
        code: "BracketRight",
        text: None,
    },
    KeyDefinition {
        key: "\"",
        key_code: 222,
        code: "Quote",
        text: None,
    },
];

/// Returns the `KeyDefinition` by key name
/// # Example return the `Enter` key: `get_key_definition("Enter").unwrap()`
pub fn get_key_definition(key: impl AsRef<str>) -> Option<&'static KeyDefinition> {
    let key = key.as_ref();
    USKEYBOARD_LAYOUT
        .iter()
        .find(|key_definition| key_definition.key == key)
}

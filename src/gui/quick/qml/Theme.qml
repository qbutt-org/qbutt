// SPDX-License-Identifier: GPL-2.0-or-later
import QtQuick

QtObject {
    required property color background
    required property color foreground

    readonly property color accent: "#009df7"
    readonly property color muted: Qt.rgba(foreground.r, foreground.g, foreground.b, 0.64)
    readonly property color hover: Qt.rgba(foreground.r, foreground.g, foreground.b, 0.055)
    readonly property color selection: Qt.rgba(accent.r, accent.g, accent.b, 0.16)
    readonly property bool dark: background.hslLightness < 0.5
    readonly property int controlHeight: 40
    readonly property int radius: 6

    function iconSource(name: string): url {
        return "qrc:/icons/" + (dark ? "dark/" : "") + name + ".svg";
    }
}

// SPDX-License-Identifier: GPL-2.0-or-later
import QtQuick

QtObject {
    required property color background
    required property color foreground

    readonly property color accent: "#009df7"
    readonly property color accentText: "#101820"
    readonly property color accentHovered: Qt.lighter(accent, 1.1)
    readonly property color accentPressed: Qt.darker(accent, 1.14)
    readonly property color muted: Qt.rgba(foreground.r, foreground.g, foreground.b, 0.64)
    readonly property color disabledText: Qt.rgba(foreground.r, foreground.g, foreground.b, 0.38)
    readonly property color hover: Qt.rgba(foreground.r, foreground.g, foreground.b, 0.055)
    readonly property color selection: Qt.rgba(accent.r, accent.g, accent.b, 0.16)

    readonly property color controlBackground: Qt.tint(background, Qt.rgba(foreground.r, foreground.g, foreground.b, 0.045))
    readonly property color controlHovered: Qt.tint(background, Qt.rgba(foreground.r, foreground.g, foreground.b, 0.08))
    readonly property color controlPressed: Qt.tint(background, Qt.rgba(foreground.r, foreground.g, foreground.b, 0.13))
    readonly property color controlBorder: Qt.rgba(foreground.r, foreground.g, foreground.b, 0.2)
    readonly property color controlBorderHovered: Qt.rgba(foreground.r, foreground.g, foreground.b, 0.36)

    readonly property bool dark: background.hslLightness < 0.5
    readonly property int controlHeight: 42
    readonly property int controlFontSize: 14
    readonly property int radius: 8

    function iconSource(name: string): url {
        return "qrc:/icons/" + (dark ? "dark/" : "") + name + ".svg";
    }
}

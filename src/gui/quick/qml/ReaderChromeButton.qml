// SPDX-License-Identifier: GPL-2.0-or-later
import QtQuick
import QtQuick.Controls.Basic

Button {
    id: control

    required property Theme theme
    required property color foreground
    property real leftCornerRadius: theme.radius
    property real rightCornerRadius: theme.radius

    implicitHeight: theme.controlHeight
    horizontalPadding: 12
    hoverEnabled: true
    font.pixelSize: theme.controlFontSize
    icon.width: 20
    icon.height: 20
    icon.color: foreground
    spacing: 8
    palette.buttonText: enabled ? foreground
        : Qt.rgba(foreground.r, foreground.g, foreground.b, 0.38)

    background: Rectangle {
        topLeftRadius: control.leftCornerRadius
        bottomLeftRadius: control.leftCornerRadius
        topRightRadius: control.rightCornerRadius
        bottomRightRadius: control.rightCornerRadius
        color: control.down
            ? Qt.rgba(control.foreground.r, control.foreground.g, control.foreground.b, 0.12)
            : control.hovered
                ? Qt.rgba(control.foreground.r, control.foreground.g, control.foreground.b, 0.07)
                : "transparent"

        border.width: control.visualFocus ? 2 : 0
        border.color: control.theme.accent
    }
}

// SPDX-License-Identifier: GPL-2.0-or-later
import QtQuick
import QtQuick.Controls.Basic

Button {
    id: control

    required property Theme theme
    required property color swatchColor

    implicitWidth: 42
    implicitHeight: 42
    hoverEnabled: true

    background: Rectangle {
        radius: control.theme.radius
        color: control.swatchColor
        border.width: control.hovered || control.visualFocus ? 2 : 1
        border.color: control.hovered || control.visualFocus
            ? control.theme.accent : control.theme.controlBorder
    }
}

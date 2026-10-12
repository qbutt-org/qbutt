// SPDX-License-Identifier: GPL-2.0-or-later
import QtQuick
import QtQuick.Controls.Basic

ToolButton {
    id: control

    required property Theme theme

    display: AbstractButton.IconOnly
    icon.width: 20
    icon.height: 20
    icon.color: "transparent"
    hoverEnabled: true
    opacity: enabled ? 1 : 0.45

    ToolTip {
        parent: control
        visible: control.hovered
        text: control.text
    }

    background: Rectangle {
        implicitWidth: control.theme.controlHeight
        implicitHeight: control.theme.controlHeight
        radius: control.theme.radius
        color: control.down ? control.theme.controlPressed
            : control.hovered ? control.theme.hover : "transparent"
        border.width: control.visualFocus ? 2 : 0
        border.color: control.theme.accent
    }
}

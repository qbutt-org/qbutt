// SPDX-License-Identifier: GPL-2.0-or-later
import QtQuick
import QtQuick.Controls.Basic

TabButton {
    id: control

    required property Theme theme

    width: implicitWidth
    horizontalPadding: 14
    hoverEnabled: true
    font.pixelSize: theme.controlFontSize
    font.weight: Font.DemiBold
    palette.windowText: theme.foreground
    palette.brightText: theme.muted

    background: Rectangle {
        implicitHeight: control.theme.controlHeight
        radius: control.theme.radius
        color: control.down ? control.theme.controlPressed
            : control.checked ? control.theme.selection
            : control.hovered ? control.theme.hover : "transparent"
        border.width: control.visualFocus ? 2 : 0
        border.color: control.theme.accent
    }
}

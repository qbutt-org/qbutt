// SPDX-License-Identifier: GPL-2.0-or-later
import QtQuick
import QtQuick.Controls.Basic

Button {
    id: control

    required property Theme theme
    required property color swatchColor
    readonly property color previewTextColor: swatchColor.hslLightness > 0.5 ? "#202520" : "#ffffff"

    implicitWidth: 42
    implicitHeight: 42
    padding: 0
    hoverEnabled: true
    font.pixelSize: 20
    font.family: "Georgia"

    contentItem: Text {
        text: control.text
        font: control.font
        color: control.previewTextColor
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
    }

    background: Rectangle {
        radius: control.theme.radius
        color: control.down
            ? Qt.tint(control.swatchColor, Qt.rgba(control.previewTextColor.r,
                control.previewTextColor.g, control.previewTextColor.b, 0.12))
            : control.swatchColor
        border.width: control.checked || control.hovered || control.visualFocus ? 2 : 1
        border.color: control.checked || control.hovered || control.visualFocus
            ? control.theme.accent : control.theme.controlBorder
    }
}

// SPDX-License-Identifier: GPL-2.0-or-later
import QtQuick
import QtQuick.Controls.Basic

Button {
    id: control

    required property Theme theme

    horizontalPadding: 16
    spacing: 8
    hoverEnabled: true
    font.pixelSize: theme.controlFontSize
    font.weight: Font.DemiBold
    icon.width: 20
    icon.height: 20
    palette.buttonText: enabled ? theme.foreground : theme.disabledText
    palette.brightText: enabled ? theme.accentText : theme.disabledText

    background: Rectangle {
        implicitWidth: 100
        implicitHeight: control.theme.controlHeight
        radius: control.theme.radius
        color: {
            if (!control.enabled)
                return control.theme.controlBackground;

            if (control.highlighted || control.checked) {
                if (control.down)
                    return control.theme.accentPressed;

                return control.hovered ? control.theme.accentHovered : control.theme.accent;
            }

            if (control.down)
                return control.theme.controlPressed;

            return control.hovered ? control.theme.controlHovered : control.theme.controlBackground;
        }
        border.width: 1
        border.color: {
            if (control.enabled && (control.highlighted || control.checked))
                return "transparent";

            return control.hovered && control.enabled ? control.theme.controlBorderHovered : control.theme.controlBorder;
        }

        Rectangle {
            anchors.fill: parent
            anchors.margins: -4
            visible: control.visualFocus
            color: "transparent"
            radius: control.theme.radius + 4
            border.width: 2
            border.color: control.theme.accent
        }
    }
}

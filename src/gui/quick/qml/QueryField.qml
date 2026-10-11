// SPDX-License-Identifier: GPL-2.0-or-later
import QtQuick
import QtQuick.Controls.Basic

TextField {
    id: control

    required property Theme theme

    padding: 10
    leftPadding: 14
    rightPadding: 14
    hoverEnabled: true
    font.pixelSize: theme.controlFontSize
    color: enabled ? theme.foreground : theme.disabledText
    placeholderTextColor: enabled ? theme.muted : theme.disabledText
    selectionColor: theme.accent
    selectedTextColor: theme.accentText

    background: Rectangle {
        implicitWidth: 200
        implicitHeight: control.theme.controlHeight
        radius: control.theme.radius
        color: control.theme.controlBackground
        border.width: control.activeFocus ? 2 : 1
        border.color: {
            if (control.activeFocus)
                return control.theme.accent;

            return control.hovered && control.enabled ? control.theme.controlBorderHovered : control.theme.controlBorder;
        }
    }
}

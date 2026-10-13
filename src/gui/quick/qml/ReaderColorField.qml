// SPDX-License-Identifier: GPL-2.0-or-later
import QtQuick
import QtQuick.Controls.Basic

Control {
    id: control

    required property Theme theme
    required property real hue
    required property real saturation
    required property real brightness
    signal changed(real saturation, real brightness)

    function selectAt(x: real, y: real): void {
        changed(Math.max(0, Math.min(1, x / width)),
            1 - Math.max(0, Math.min(1, y / height)));
    }

    implicitHeight: 204
    padding: 0
    activeFocusOnTab: true
    Accessible.role: Accessible.Slider
    Accessible.name: qsTr("Saturation and brightness")
    Keys.onLeftPressed: changed(Math.max(0, saturation - 0.01), brightness)
    Keys.onRightPressed: changed(Math.min(1, saturation + 0.01), brightness)
    Keys.onUpPressed: changed(saturation, Math.min(1, brightness + 0.01))
    Keys.onDownPressed: changed(saturation, Math.max(0, brightness - 0.01))

    background: Rectangle {
        radius: control.theme.radius
        color: Qt.hsva(control.hue, 1, 1, 1)

        Rectangle {
            anchors.fill: parent
            radius: control.theme.radius
            gradient: Gradient {
                orientation: Gradient.Horizontal
                GradientStop {
                    position: 0
                    color: "#ffffff"
                }
                GradientStop {
                    position: 1
                    color: "#00ffffff"
                }
            }
        }

        Rectangle {
            anchors.fill: parent
            radius: control.theme.radius
            gradient: Gradient {
                GradientStop {
                    position: 0
                    color: "#00000000"
                }
                GradientStop {
                    position: 1
                    color: "#000000"
                }
            }
        }

        Rectangle {
            anchors.fill: parent
            radius: control.theme.radius
            color: "transparent"
            border.width: control.visualFocus ? 2 : 1
            border.color: control.visualFocus ? control.theme.accent : control.theme.controlBorder
        }
    }

    contentItem: Item {
        Rectangle {
            width: 20
            height: 20
            x: Math.max(2, Math.min(parent.width - width - 2,
                control.saturation * parent.width - width / 2))
            y: Math.max(2, Math.min(parent.height - height - 2,
                (1 - control.brightness) * parent.height - height / 2))
            radius: width / 2
            color: "transparent"
            border.width: 2
            border.color: "#ffffff"

            Rectangle {
                anchors.fill: parent
                anchors.margins: 2
                radius: width / 2
                color: "transparent"
                border.width: 1
                border.color: "#80000000"
            }
        }

        MouseArea {
            id: pointer
            anchors.fill: parent
            onPressed: mouse => {
                control.forceActiveFocus(Qt.MouseFocusReason);
                control.selectAt(mouse.x, mouse.y);
            }
            onPositionChanged: mouse => {
                if (pointer.pressed)
                    control.selectAt(mouse.x, mouse.y);
            }
        }
    }
}

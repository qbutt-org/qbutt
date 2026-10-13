// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts

ModalPicker {
    id: root
    objectName: "readerColorEditor"

    required property string title
    property real hue: 0
    property real saturation: 0
    property real brightness: 1
    readonly property color draftColor: Qt.hsva(hue, saturation, brightness, 1)
    signal colorSelected(color color)

    function selectColor(color: color): void {
        hue = Math.max(0, color.hsvHue);
        saturation = color.hsvSaturation;
        brightness = color.hsvValue;
    }

    function openColor(color: color): void {
        selectColor(color);
        hex.text = Qt.binding(() => root.draftColor.toString().toUpperCase());
        open();
        field.forceActiveFocus();
    }

    width: 396
    height: contentItem.implicitHeight + topPadding + bottomPadding

    contentItem: ColumnLayout {
        spacing: 16

        Label {
            Layout.fillWidth: true
            text: root.title
            color: root.theme.foreground
            font.pixelSize: 22
            font.weight: Font.DemiBold
        }

        ReaderColorField {
            id: field
            objectName: "readerColorField"
            Layout.fillWidth: true
            theme: root.theme
            hue: root.hue
            saturation: root.saturation
            brightness: root.brightness
            onChanged: (saturation, brightness) => {
                root.saturation = saturation;
                root.brightness = brightness;
            }
        }

        Slider {
            id: hueSlider
            objectName: "readerColorHue"
            Layout.fillWidth: true
            implicitHeight: 32
            from: 0
            to: 1
            value: root.hue
            Accessible.name: qsTr("Hue")
            onMoved: root.hue = value

            background: Rectangle {
                x: hueSlider.leftPadding + hueSlider.handle.width / 2
                y: hueSlider.topPadding + (hueSlider.availableHeight - height) / 2
                width: hueSlider.availableWidth - hueSlider.handle.width
                height: 12
                radius: height / 2
                gradient: Gradient {
                    orientation: Gradient.Horizontal
                    GradientStop {
                        position: 0
                        color: "#ff0000"
                    }
                    GradientStop {
                        position: 1 / 6
                        color: "#ffff00"
                    }
                    GradientStop {
                        position: 2 / 6
                        color: "#00ff00"
                    }
                    GradientStop {
                        position: 3 / 6
                        color: "#00ffff"
                    }
                    GradientStop {
                        position: 4 / 6
                        color: "#0000ff"
                    }
                    GradientStop {
                        position: 5 / 6
                        color: "#ff00ff"
                    }
                    GradientStop {
                        position: 1
                        color: "#ff0000"
                    }
                }
            }

            handle: Rectangle {
                x: hueSlider.leftPadding + hueSlider.visualPosition * (hueSlider.availableWidth - width)
                y: hueSlider.topPadding + (hueSlider.availableHeight - height) / 2
                width: 24
                height: 24
                radius: width / 2
                color: Qt.hsva(root.hue, 1, 1, 1)
                border.width: 3
                border.color: hueSlider.visualFocus ? root.theme.accent : "#ffffff"
            }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 12

            Rectangle {
                Layout.preferredWidth: 42
                Layout.preferredHeight: 42
                radius: root.theme.radius
                color: root.draftColor
                border.width: 1
                border.color: root.theme.controlBorder
            }

            QueryField {
                id: hex
                objectName: "readerColorHex"
                Layout.fillWidth: true
                theme: root.theme
                text: root.draftColor.toString().toUpperCase()
                placeholderText: "#RRGGBB"
                Accessible.name: qsTr("Hex color")
                maximumLength: 7
                selectByMouse: true
                inputMethodHints: Qt.ImhNoPredictiveText | Qt.ImhUppercaseOnly
                validator: RegularExpressionValidator {
                    regularExpression: /#?[0-9a-fA-F]{6}/
                }
                onTextEdited: {
                    if (acceptableInput)
                        root.selectColor("#" + text.replace("#", ""));
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 12

            ActionButton {
                objectName: "readerColorCancel"
                Layout.fillWidth: true
                theme: root.theme
                text: qsTr("Cancel")
                onClicked: root.close()
            }

            ActionButton {
                objectName: "readerColorApply"
                Layout.fillWidth: true
                theme: root.theme
                text: qsTr("Apply")
                highlighted: true
                enabled: hex.acceptableInput
                onClicked: {
                    root.colorSelected(root.draftColor);
                    root.close();
                }
            }
        }
    }
}

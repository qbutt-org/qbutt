// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Dialogs as Dialogs
import QtQuick.Layouts
import Qbutt.Content

ModalPicker {
    id: root
    objectName: "readerAppearancePicker"

    required property ContentController backend
    required property color effectiveBackground
    required property color effectiveTextColor
    signal appearanceChanging
    signal fitRequested(int fit)
    signal modeRequested(int mode)

    anchors.centerIn: undefined
    width: Math.min(420, parent.width - 48)
    height: Math.min(backend.readerIsText ? 570 : 350, parent.height - 48)
    x: parent.width - width - 24
    y: (parent.height - height) / 2
    palette.base: theme.controlBackground
    palette.button: theme.controlBackground
    palette.mid: theme.controlPressed

    contentItem: ColumnLayout {
        spacing: 12

        RowLayout {
            Layout.fillWidth: true

            Label {
                Layout.fillWidth: true
                text: qsTr("Reading appearance")
                font.pixelSize: 22
                font.weight: Font.DemiBold
            }

            ReaderChromeButton {
                objectName: "readerAppearanceClose"
                theme: root.theme
                foreground: root.theme.foreground
                icon.source: root.theme.iconSource("close")
                horizontalPadding: 0
                Layout.preferredWidth: 42
                Accessible.name: qsTr("Close")
                onClicked: root.close()
            }
        }

        ScrollView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            contentWidth: availableWidth
            clip: true

            ColumnLayout {
                width: parent.width
                spacing: 12

                RowLayout {
                    Layout.fillWidth: true

                    Label {
                        Layout.fillWidth: true
                        text: qsTr("Background")
                    }

                    Label {
                        text: root.backend.readerBackgroundColor === ""
                            ? qsTr("Default") : root.backend.readerBackgroundColor
                        color: root.theme.muted
                    }

                    ReaderColorSwatch {
                        objectName: "readerBackgroundPicker"
                        theme: root.theme
                        swatchColor: root.effectiveBackground
                        Accessible.name: qsTr("Change background color")
                        onClicked: {
                            backgroundDialog.selectedColor = root.effectiveBackground;
                            backgroundDialog.open();
                        }
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    visible: root.backend.readerIsText

                    Label {
                        Layout.fillWidth: true
                        text: qsTr("Text color")
                    }

                    Label {
                        text: root.backend.readerTextColor === ""
                            ? qsTr("Default") : root.backend.readerTextColor
                        color: root.theme.muted
                    }

                    ReaderColorSwatch {
                        objectName: "readerTextColorPicker"
                        theme: root.theme
                        swatchColor: root.effectiveTextColor
                        Accessible.name: qsTr("Change text color")
                        onClicked: {
                            textDialog.selectedColor = root.effectiveTextColor;
                            textDialog.open();
                        }
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    visible: root.backend.readerIsText

                    Label {
                        Layout.fillWidth: true
                        text: qsTr("Font")
                    }

                    ActionButton {
                        id: fontButton
                        objectName: "readerFontPickerButton"
                        Layout.preferredWidth: 189
                        theme: root.theme
                        horizontalPadding: 14
                        text: root.backend.readerFontFamily !== ""
                            ? root.backend.readerFontFamily : qsTr("System")
                        Accessible.name: qsTr("Font: %1").arg(text)
                        onClicked: fonts.open()

                        contentItem: RowLayout {
                            spacing: 8

                            Label {
                                Layout.fillWidth: true
                                text: fontButton.text
                                font.family: root.backend.readerFontFamily !== ""
                                    ? root.backend.readerFontFamily : fontButton.font.family
                                font.pixelSize: fontButton.font.pixelSize
                                color: fontButton.enabled
                                    ? root.theme.foreground : root.theme.disabledText
                                elide: Text.ElideRight
                                verticalAlignment: Text.AlignVCenter
                            }

                            Image {
                                Layout.preferredWidth: 18
                                Layout.preferredHeight: 18
                                source: root.theme.iconSource("chevron-down")
                                sourceSize: Qt.size(18, 18)
                                fillMode: Image.PreserveAspectFit
                                Accessible.ignored: true
                            }
                        }
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    visible: root.backend.readerIsText

                    Label {
                        Layout.fillWidth: true
                        text: qsTr("Font size")
                    }

                    SpinBox {
                        objectName: "readerFontSize"
                        from: 12
                        to: 36
                        value: root.backend.readerFontSize
                        editable: true
                        Accessible.name: qsTr("Font size")
                        onValueModified: {
                            root.appearanceChanging();
                            root.backend.setReaderFontSize(value);
                        }
                    }
                }

                Label {
                    visible: root.backend.readerIsText
                    text: qsTr("Advanced settings")
                    font.weight: Font.DemiBold
                    color: root.theme.foreground
                    topPadding: 14
                }

                RowLayout {
                    Layout.fillWidth: true
                    visible: root.backend.readerIsText

                    Label {
                        Layout.fillWidth: true
                        text: qsTr("Line height")
                    }

                    SpinBox {
                        objectName: "readerLineHeight"
                        from: 110
                        to: 200
                        stepSize: 5
                        value: root.backend.readerLineHeight
                        editable: true
                        Accessible.name: qsTr("Line height, percent")
                        textFromValue: value => value + "%"
                        valueFromText: text => parseInt(text)
                        onValueModified: {
                            root.appearanceChanging();
                            root.backend.setReaderLineHeight(value);
                        }
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    visible: root.backend.readerIsText

                    Label {
                        Layout.fillWidth: true
                        text: qsTr("Text width")
                    }

                    SpinBox {
                        objectName: "readerColumnWidth"
                        from: 480
                        to: 1040
                        stepSize: 40
                        value: root.backend.readerColumnWidth
                        editable: true
                        Accessible.name: qsTr("Text width, pixels")
                        onValueModified: {
                            root.appearanceChanging();
                            root.backend.setReaderColumnWidth(value);
                        }
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    visible: !root.backend.readerIsText

                    Label {
                        Layout.fillWidth: true
                        text: qsTr("Reading mode")
                    }

                    SelectField {
                        objectName: "readerMode"
                        Layout.preferredWidth: 189
                        theme: root.theme
                        textRole: "title"
                        valueRole: "mode"
                        model: [
                            { title: qsTr("Pages"), mode: Reader.Paged },
                            { title: qsTr("Continuous"), mode: Reader.Continuous }
                        ]
                        currentIndex: root.backend.readerMode === Reader.Continuous ? 1 : 0
                        onActivated: root.modeRequested(currentValue)
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    visible: !root.backend.readerIsText

                    Label {
                        Layout.fillWidth: true
                        text: qsTr("Fit")
                    }

                    ActionButton {
                        objectName: "readerHeightFit"
                        theme: root.theme
                        text: qsTr("Height")
                        Accessible.name: qsTr("Fit to height")
                        checked: root.backend.readerFit === Reader.Height
                        onClicked: root.fitRequested(Reader.Height)
                    }

                    ActionButton {
                        objectName: "readerWidthFit"
                        theme: root.theme
                        text: qsTr("Width")
                        Accessible.name: qsTr("Fit to width")
                        checked: root.backend.readerFit === Reader.Width
                        onClicked: root.fitRequested(Reader.Width)
                    }
                }

                ReaderChromeButton {
                    objectName: "readerAppearanceReset"
                    Layout.topMargin: 12
                    theme: root.theme
                    foreground: root.theme.muted
                    text: qsTr("Reset to defaults")
                    onClicked: {
                        root.appearanceChanging();
                        root.backend.resetReaderAppearance();
                    }
                }
            }
        }
    }

    ReaderFontPicker {
        id: fonts
        theme: root.theme
        backdrop: root.backdrop
        selectedFamily: root.backend.readerFontFamily
        onFontSelected: family => {
            root.appearanceChanging();
            root.backend.setReaderFontFamily(family);
        }
    }

    // QQuickWidget's implicit dialog parent is the hosting QWidget window.
    Dialogs.ColorDialog {
        id: backgroundDialog
        title: qsTr("Background")
        options: Dialogs.ColorDialog.DontUseNativeDialog | Dialogs.ColorDialog.NoEyeDropperButton
        popupType: Popup.Item
        parentWindow: root.backdrop.Window.window
        onAccepted: root.backend.setReaderBackgroundColor(selectedColor.toString())
    }

    Dialogs.ColorDialog {
        id: textDialog
        title: qsTr("Text color")
        options: Dialogs.ColorDialog.DontUseNativeDialog | Dialogs.ColorDialog.NoEyeDropperButton
        popupType: Popup.Item
        parentWindow: root.backdrop.Window.window
        onAccepted: root.backend.setReaderTextColor(selectedColor.toString())
    }
}

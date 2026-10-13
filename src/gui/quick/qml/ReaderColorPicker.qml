// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts

Popup {
    id: root
    objectName: "readerColorPicker"

    required property Theme theme
    required property Item backdrop
    required property color selectedColor
    required property bool defaultSelected
    property bool textMode: false
    readonly property var presets: textMode
        ? ["#202520", "#414b43", "#6d5040", "#255166",
           "#ffffff", "#e7e9e3", "#e6dbc6", "#b5c6d4"]
        : ["#ffffff", "#f7f5f0", "#f2e7d5", "#e8ede4",
           "#dde7ed", "#313b43", "#252925", "#15191e"]
    signal colorSelected(string color)

    function openFor(anchor: Item, forText: bool): void {
        textMode = forText;
        const position = anchor.mapToItem(parent, 0, 0);
        x = Math.max(margins, Math.min(position.x + anchor.width - width,
            parent.width - width - margins));
        const below = position.y + anchor.height + 8;
        y = below + height <= parent.height - margins ? below : position.y - height - 8;
        open();
    }

    function dismiss(): void {
        close();
        editor.close();
    }

    parent: Overlay.overlay
    width: 296
    height: contentItem.implicitHeight + topPadding + bottomPadding
    padding: 12
    margins: 12
    modal: true
    dim: false
    focus: true
    popupType: Popup.Item
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

    background: Rectangle {
        radius: root.theme.radius + 4
        color: root.theme.background
        border.width: 1
        border.color: root.theme.controlBorder
    }

    contentItem: ColumnLayout {
        spacing: 12

        GridLayout {
            Layout.fillWidth: true
            columns: 4
            rowSpacing: 8
            columnSpacing: 8

            Repeater {
                model: root.presets

                ReaderColorSwatch {
                    required property string modelData
                    required property int index

                    objectName: "readerColorPreset" + index
                    Layout.fillWidth: true
                    Layout.preferredHeight: 48
                    theme: root.theme
                    swatchColor: modelData
                    text: qsTr("Aa")
                    checked: !root.defaultSelected && Qt.colorEqual(root.selectedColor, swatchColor)
                    Accessible.name: qsTr("Select %1").arg(modelData)
                    onClicked: {
                        root.colorSelected(modelData);
                        root.close();
                    }
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 8

            ReaderChromeButton {
                objectName: "readerColorDefault"
                theme: root.theme
                foreground: root.theme.foreground
                horizontalPadding: 8
                text: qsTr("Default")
                enabled: !root.defaultSelected
                onClicked: {
                    root.colorSelected("");
                    root.close();
                }
            }

            ActionButton {
                objectName: "readerColorCustom"
                Layout.fillWidth: true
                theme: root.theme
                horizontalPadding: 10
                text: qsTr("Custom color…")
                onClicked: {
                    editor.openColor(root.selectedColor);
                    root.close();
                }
            }
        }
    }

    ReaderColorEditor {
        id: editor
        theme: root.theme
        backdrop: root.backdrop
        title: root.textMode ? qsTr("Text color") : qsTr("Background")
        onColorSelected: color => root.colorSelected(color.toString())
    }
}

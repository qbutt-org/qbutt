// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts

ActionButton {
    id: root

    required property Item backdrop
    required property string title
    required property string allText
    required property string value
    required property var options
    property string fallbackText: value
    signal chosen(var option)

    readonly property var selectedOption: options.find(option => option.value === value)
    text: value.length === 0 ? allText : selectedOption ? selectedOption.label : fallbackText
    Accessible.name: title + ": " + text
    highlighted: value.length > 0
    icon.source: theme.iconSource("chevron-down")
    onClicked: picker.open()

    ModalPicker {
        id: picker
        objectName: root.objectName + "Picker"
        theme: root.theme
        backdrop: root.backdrop
        onOpened: query.forceActiveFocus()
        onClosed: root.forceActiveFocus()

        contentItem: ColumnLayout {
            spacing: 16

            RowLayout {
                spacing: 16

                Label {
                    Layout.fillWidth: true
                    text: root.title
                    font.pixelSize: 24
                    font.weight: Font.DemiBold
                }

                ActionButton {
                    theme: root.theme
                    text: qsTr("Close")
                    onClicked: picker.close()
                }
            }

            QueryField {
                id: query
                objectName: root.objectName + "Query"
                Layout.fillWidth: true
                theme: root.theme
                placeholderText: qsTr("Find")
                Accessible.name: placeholderText
                onAccepted: choices.forceActiveFocus()
            }

            ListView {
                id: choices
                objectName: root.objectName + "Choices"
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                spacing: 4
                activeFocusOnTab: true
                reuseItems: true
                model: [{ value: "", label: root.allText }].concat(root.options)
                    .filter(option => option.label.toLocaleLowerCase().includes(query.text.toLocaleLowerCase()))
                currentIndex: model.findIndex(option => option.value === root.value)
                ScrollBar.vertical: ScrollBar {}

                function choose(option: var): void {
                    root.chosen(option);
                    picker.close();
                }

                Keys.onReturnPressed: {
                    if (currentIndex >= 0)
                        choose(model[currentIndex]);
                }
                Keys.onEnterPressed: {
                    if (currentIndex >= 0)
                        choose(model[currentIndex]);
                }

                delegate: ItemDelegate {
                    id: option
                    required property var modelData
                    width: ListView.view.width
                    padding: 12
                    focusPolicy: Qt.NoFocus
                    highlighted: modelData.value === root.value
                    Accessible.selected: highlighted
                    text: modelData.label
                    onClicked: choices.choose(modelData)

                    background: Rectangle {
                        radius: root.theme.radius
                        color: option.highlighted ? root.theme.selection : option.hovered ? root.theme.hover : "transparent"
                        border.width: option.ListView.isCurrentItem && choices.activeFocus ? 1 : 0
                        border.color: root.theme.accent
                    }
                }

                Label {
                    anchors.centerIn: parent
                    visible: choices.count === 0
                    text: qsTr("No matches")
                    color: root.theme.muted
                }
            }
        }
    }
}

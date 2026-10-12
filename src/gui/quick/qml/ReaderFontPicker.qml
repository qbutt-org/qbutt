// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts

ModalPicker {
    id: root
    objectName: "readerFontPicker"

    required property string selectedFamily
    signal fontSelected(string family)
    readonly property string systemFontLabel: qsTr("System")
    readonly property var families: [""].concat(Qt.fontFamilies())
    readonly property var matches: families.filter(family =>
        (family || systemFontLabel).toLocaleLowerCase().includes(search.text.toLocaleLowerCase()))

    onOpened: search.forceActiveFocus()

    contentItem: ColumnLayout {
        spacing: 16

        RowLayout {
            Layout.fillWidth: true

            Label {
                Layout.fillWidth: true
                text: qsTr("Font")
                font.pixelSize: 24
                font.weight: Font.DemiBold
            }

            ReaderChromeButton {
                theme: root.theme
                foreground: root.theme.foreground
                icon.source: root.theme.iconSource("close")
                horizontalPadding: 0
                Layout.preferredWidth: 42
                Accessible.name: qsTr("Close")
                onClicked: root.close()
            }
        }

        QueryField {
            id: search
            objectName: "readerFontSearch"
            Layout.fillWidth: true
            theme: root.theme
            placeholderText: qsTr("Find a font")
            Accessible.name: placeholderText
        }

        ListView {
            id: fonts
            objectName: "readerFontList"
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: root.matches
            reuseItems: true
            currentIndex: -1
            ScrollBar.vertical: ScrollBar {}

            delegate: ItemDelegate {
                id: choice
                required property string modelData

                width: ListView.view.width
                text: modelData || root.systemFontLabel
                font.family: modelData || root.font.family
                palette.text: root.theme.foreground
                highlighted: modelData === root.selectedFamily
                Accessible.selected: highlighted
                onClicked: {
                    root.fontSelected(modelData);
                    root.close();
                }

                background: Rectangle {
                    color: choice.highlighted ? root.theme.selection
                        : choice.hovered ? root.theme.hover : "transparent"
                    radius: root.theme.radius
                }
            }
        }
    }
}

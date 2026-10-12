// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Qbutt.Content

Pane {
    id: root
    objectName: "readerToolbar"

    required property ContentController backend
    required property Theme theme
    required property color foreground
    signal backRequested
    signal chaptersRequested
    signal appearanceRequested

    padding: 0
    background: Item {}

    contentItem: RowLayout {
        spacing: 18

        ReaderChromeButton {
            objectName: "readerBack"
            theme: root.theme
            foreground: root.foreground
            icon.source: root.theme.iconSource("chevron-left")
            text: qsTr("Back")
            Accessible.name: qsTr("Back")
            onClicked: root.backRequested()
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 0

            Label {
                objectName: "readerTitle"
                Layout.fillWidth: true
                text: root.backend.readerTitle
                color: root.foreground
                font.pixelSize: 17
                font.weight: Font.DemiBold
                elide: Text.ElideRight
                textFormat: Text.PlainText
            }

            ReaderChromeButton {
                id: chapter
                objectName: "readerChapters"
                Layout.maximumWidth: 360
                Layout.preferredHeight: 25
                horizontalPadding: 0
                theme: root.theme
                foreground: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.65)
                text: root.backend.readerChapterCount > 0
                    ? qsTr("Chapter %1 of %2").arg(root.backend.readerChapterIndex + 1)
                        .arg(root.backend.readerChapterCount)
                    : qsTr("Chapters")
                Accessible.name: qsTr("Choose chapter")
                Accessible.description: root.backend.readerChapterTitle
                enabled: root.backend.readerHasChapters && !root.backend.readerBusy
                onClicked: root.chaptersRequested()

                contentItem: RowLayout {
                    spacing: 6

                    Label {
                        Layout.fillWidth: true
                        text: chapter.text
                        color: chapter.foreground
                        font: chapter.font
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

                ToolTip {
                    parent: chapter
                    visible: chapter.hovered
                    text: root.backend.readerChapterTitle
                }
            }
        }

        ReaderChromeButton {
            id: appearance
            objectName: "readerAppearance"
            Layout.preferredWidth: 56
            theme: root.theme
            foreground: root.foreground
            text: root.backend.readerIsText ? qsTr("Aa") : qsTr("View")
            font.pixelSize: root.backend.readerIsText ? 21 : root.theme.controlFontSize
            Accessible.name: qsTr("Appearance")
            onClicked: root.appearanceRequested()

            ToolTip {
                parent: appearance
                visible: appearance.hovered
                text: appearance.Accessible.name
            }
        }
    }
}

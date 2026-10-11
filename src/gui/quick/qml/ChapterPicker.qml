// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Qbutt.Content

ModalPicker {
    id: root
    objectName: "readerChapterPicker"

    required property ContentController backend

    onOpened: query.forceActiveFocus()

    contentItem: ColumnLayout {
        spacing: 16

        RowLayout {
            spacing: 16

            Label {
                Layout.fillWidth: true
                text: qsTr("Chapters")
                font.pixelSize: 24
                font.weight: Font.DemiBold
            }

            ActionButton {
                objectName: "readerChapterPickerClose"
                theme: root.theme
                text: qsTr("Close")
                onClicked: root.close()
            }
        }

        QueryField {
            id: query
            objectName: "readerChapterQuery"
            Layout.fillWidth: true
            theme: root.theme
            placeholderText: qsTr("Find a chapter")
            Accessible.name: placeholderText
            selectByMouse: true
            text: root.backend.chapterQuery
            onTextEdited: root.backend.setChapterQuery(text)
            onAccepted: chapters.forceActiveFocus()
        }

        ListView {
            id: chapters
            objectName: "readerChapterList"
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: root.backend.readerChapters
            currentIndex: root.backend.readerChapters.indexOfId(root.backend.readerChapterId)
            spacing: 4
            activeFocusOnTab: true
            keyNavigationEnabled: true
            reuseItems: true
            ScrollBar.vertical: ScrollBar {}

            function chooseCurrent(): void {
                if (currentIndex >= 0) {
                    root.backend.selectChapter(root.backend.readerChapters.idAt(currentIndex));
                    root.close();
                }
            }

            Keys.onReturnPressed: chooseCurrent()
            Keys.onEnterPressed: chooseCurrent()

            delegate: ItemDelegate {
                id: chapter

                required property string chapterId
                required property string title
                required property string number
                required property string volume
                required property int pageCount

                width: ListView.view.width
                padding: 12
                focusPolicy: Qt.NoFocus
                highlighted: chapter.chapterId === root.backend.readerChapterId
                Accessible.name: chapter.number.length > 0 ? qsTr("Chapter %1").arg(chapter.number) : chapter.title
                Accessible.selected: highlighted
                onClicked: {
                    root.backend.selectChapter(chapter.chapterId);
                    root.close();
                }

                contentItem: RowLayout {
                    spacing: 16

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 4

                        Label {
                            Layout.fillWidth: true
                            text: chapter.number.length > 0 ? qsTr("Chapter %1").arg(chapter.number) : chapter.title
                            font.weight: Font.DemiBold
                            elide: Text.ElideRight
                            textFormat: Text.PlainText
                        }

                        Label {
                            Layout.fillWidth: true
                            visible: chapter.number.length > 0 && chapter.title.length > 0
                            text: chapter.title
                            elide: Text.ElideRight
                            textFormat: Text.PlainText
                        }

                        Label {
                            visible: chapter.volume.length > 0
                            text: qsTr("Volume %1").arg(chapter.volume)
                            color: root.theme.muted
                            textFormat: Text.PlainText
                        }
                    }

                    Label {
                        text: qsTr("Pages: %1").arg(chapter.pageCount)
                        color: root.theme.muted
                    }
                }

                background: Rectangle {
                    radius: root.theme.radius
                    color: chapter.highlighted ? root.theme.selection : chapter.hovered ? root.theme.hover : "transparent"
                    border.width: chapter.ListView.isCurrentItem && chapters.activeFocus ? 1 : 0
                    border.color: root.theme.accent
                }
            }

            Label {
                anchors.centerIn: parent
                visible: chapters.count === 0
                text: qsTr("No chapters found")
                color: root.theme.muted
            }
        }
    }
}

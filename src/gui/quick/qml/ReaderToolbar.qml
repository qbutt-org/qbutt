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
    signal backRequested
    signal chaptersRequested
    signal pageRequested(int index)
    signal fitRequested(int fit)
    signal modeRequested(int mode)

    padding: 0
    background: Item {}

    contentItem: ColumnLayout {
        spacing: 12

        RowLayout {
            Layout.fillWidth: true
            spacing: 12

            ActionButton {
                objectName: "readerBack"
                theme: root.theme
                text: qsTr("Back")
                onClicked: root.backRequested()
            }

            Label {
                objectName: "readerTitle"
                Layout.fillWidth: true
                text: root.backend.readerTitle
                font.pixelSize: 20
                font.weight: Font.DemiBold
                elide: Text.ElideRight
                textFormat: Text.PlainText
            }

            ActionButton {
                id: chapter
                objectName: "readerChapters"
                Layout.preferredWidth: 240
                theme: root.theme
                icon.source: root.theme.iconSource("go-down")
                text: root.backend.readerChapterTitle !== "" ? root.backend.readerChapterTitle : qsTr("Chapters")
                Accessible.name: qsTr("Choose chapter")
                Accessible.description: text
                enabled: root.backend.readerHasChapters && !root.backend.readerBusy
                onClicked: root.chaptersRequested()

                ToolTip {
                    parent: chapter
                    visible: chapter.hovered
                    text: chapter.text
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 12

            SelectField {
                objectName: "readerMode"
                Layout.preferredWidth: 176
                theme: root.theme
                Accessible.name: qsTr("Reading mode")
                textRole: "title"
                valueRole: "mode"
                model: [
                    {
                        title: qsTr("Pages"),
                        mode: Reader.Paged
                    },
                    {
                        title: qsTr("Continuous"),
                        mode: Reader.Continuous
                    }
                ]
                currentIndex: root.backend.readerMode === Reader.Continuous ? 1 : 0
                onActivated: root.modeRequested(currentValue)
            }

            RowLayout {
                spacing: 8

                ActionButton {
                    id: heightFit
                    objectName: "readerHeightFit"
                    theme: root.theme
                    text: qsTr("Height")
                    Accessible.name: qsTr("Fit to height")
                    checked: root.backend.readerFit === Reader.Height
                    onClicked: root.fitRequested(Reader.Height)

                    ToolTip {
                        parent: heightFit
                        visible: heightFit.hovered
                        text: qsTr("Fit to height")
                    }
                }

                ActionButton {
                    id: widthFit
                    objectName: "readerWidthFit"
                    theme: root.theme
                    text: qsTr("Width")
                    Accessible.name: qsTr("Fit to width")
                    checked: root.backend.readerFit === Reader.Width
                    onClicked: root.fitRequested(Reader.Width)

                    ToolTip {
                        parent: widthFit
                        visible: widthFit.hovered
                        text: qsTr("Fit to width")
                    }
                }
            }

            Item {
                Layout.fillWidth: true
            }

            RowLayout {
                spacing: 8

                ActionButton {
                    id: previous
                    objectName: "readerPreviousPage"
                    Layout.preferredWidth: root.theme.controlHeight
                    theme: root.theme
                    text: "‹"
                    font.pixelSize: 24
                    horizontalPadding: 0
                    Accessible.name: qsTr("Previous page")
                    enabled: root.backend.readerPageIndex > 0 && !root.backend.readerBusy
                    onClicked: root.pageRequested(root.backend.readerPageIndex - 1)

                    ToolTip {
                        parent: previous
                        visible: previous.hovered
                        text: qsTr("Previous page")
                    }
                }

                Label {
                    objectName: "readerPageNumber"
                    Layout.preferredWidth: Math.max(116, implicitWidth)
                    text: root.backend.readerPageCount > 0 ? qsTr("Page %1 of %2").arg(root.backend.readerPageIndex + 1).arg(root.backend.readerPageCount) : ""
                    horizontalAlignment: Text.AlignHCenter
                }

                ActionButton {
                    id: next
                    objectName: "readerNextPage"
                    Layout.preferredWidth: root.theme.controlHeight
                    theme: root.theme
                    text: "›"
                    font.pixelSize: 24
                    horizontalPadding: 0
                    Accessible.name: qsTr("Next page")
                    enabled: root.backend.readerPageIndex + 1 < root.backend.readerPageCount && !root.backend.readerBusy
                    onClicked: root.pageRequested(root.backend.readerPageIndex + 1)

                    ToolTip {
                        parent: next
                        visible: next.hovered
                        text: qsTr("Next page")
                    }
                }
            }
        }
    }
}

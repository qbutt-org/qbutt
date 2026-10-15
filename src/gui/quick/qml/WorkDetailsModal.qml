// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Qbutt.Content

ModalPicker {
    id: root
    objectName: "contentWorkDetailsModal"

    required property ContentController backend
    readonly property string publicationId: backend.selectedPublicationId
    readonly property var notes: backend.selectedPublicationNotes

    onPublicationIdChanged: {
        close();
        document.cancelFlick();
        document.contentY = 0;
    }

    contentItem: ColumnLayout {
        spacing: 16

        RowLayout {
            spacing: 16

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 4

                Label {
                    Layout.fillWidth: true
                    text: root.backend.selectedWorkTitle
                    font.pixelSize: 24
                    font.weight: Font.DemiBold
                    wrapMode: Text.Wrap
                    maximumLineCount: 2
                    elide: Text.ElideRight
                    textFormat: Text.PlainText
                }

                Label {
                    objectName: "contentWorkDetailsSource"
                    text: root.backend.selectedPublicationSource
                    color: root.theme.muted
                    textFormat: Text.PlainText
                }
            }

            ActionButton {
                objectName: "contentWorkDetailsClose"
                Layout.alignment: Qt.AlignTop
                theme: root.theme
                text: qsTr("Close")
                onClicked: root.close()
            }
        }

        ScrollView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            contentItem: Flickable {
                id: document
                objectName: "contentWorkDetailsDocument"
                contentWidth: width
                contentHeight: publication.implicitHeight
                boundsBehavior: Flickable.StopAtBounds
                acceptedButtons: Qt.NoButton
                clip: true

                Column {
                    id: publication
                    width: document.width
                    spacing: 24

                    TextArea {
                        objectName: "contentWorkDetailsText"
                        width: publication.width
                        visible: text.length > 0
                        text: root.backend.selectedWorkDescription
                        color: root.theme.foreground
                        wrapMode: TextEdit.Wrap
                        textFormat: TextEdit.PlainText
                        readOnly: true
                        selectByMouse: true
                        padding: 0
                        background: null
                    }

                    Label {
                        visible: root.notes.length > 0
                        text: qsTr("Comments")
                        font.pixelSize: 20
                        font.weight: Font.DemiBold
                    }

                    Repeater {
                        model: root.notes

                        delegate: ColumnLayout {
                            id: comment
                            required property var modelData
                            required property int index

                            width: publication.width
                            spacing: 4

                            RowLayout {
                                Layout.fillWidth: true

                                Label {
                                    objectName: "contentPublicationNoteAuthor"
                                    Layout.fillWidth: true
                                    visible: text.length > 0
                                    text: comment.modelData.author
                                    font.weight: Font.DemiBold
                                    wrapMode: Text.Wrap
                                    textFormat: Text.PlainText
                                }

                                ToolButton {
                                    objectName: "contentPublicationNoteOpen"
                                    visible: comment.modelData.hasSourceLink
                                    text: qsTr("Open comment")
                                    palette.buttonText: root.theme.accent
                                    onClicked: root.backend.openPublicationNote(root.publicationId, comment.index)
                                }
                            }

                            TextArea {
                                objectName: "contentPublicationNoteText"
                                Layout.fillWidth: true
                                text: comment.modelData.text
                                color: root.theme.foreground
                                wrapMode: TextEdit.Wrap
                                textFormat: TextEdit.PlainText
                                readOnly: true
                                selectByMouse: true
                                padding: 0
                                background: null
                            }
                        }
                    }
                }
            }
        }
    }
}

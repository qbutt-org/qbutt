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
                contentHeight: instructions.implicitHeight
                boundsBehavior: Flickable.StopAtBounds
                acceptedButtons: Qt.NoButton
                clip: true

                TextArea {
                    id: instructions
                    objectName: "contentWorkDetailsText"
                    width: document.width
                    text: root.backend.selectedWorkDescription
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

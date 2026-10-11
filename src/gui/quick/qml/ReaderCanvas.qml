// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Qbutt.Content

Item {
    id: root
    objectName: "readerCanvas"

    required property ContentController backend
    required property Theme theme
    readonly property int fit: backend.readerFit
    readonly property string errorText: {
        if (backend.readerErrorText.length > 0)
            return backend.readerErrorText;

        return page.status === Image.Error ? qsTr("Could not display this page.") : "";
    }
    onFitChanged: viewport.contentY = 0

    ScrollView {
        objectName: "readerPageScroll"
        anchors.fill: parent
        clip: true
        contentWidth: availableWidth
        visible: root.errorText.length === 0

        Flickable {
            id: viewport
            objectName: "readerPageViewport"
            contentWidth: width
            contentHeight: Math.max(height, page.height)
            boundsBehavior: Flickable.StopAtBounds
            flickableDirection: Flickable.VerticalFlick

            Image {
                id: page
                objectName: "readerPageImage"
                width: viewport.width
                height: {
                    if (root.fit === Reader.Height || implicitWidth <= 0)
                        return viewport.height;

                    return width * implicitHeight / implicitWidth;
                }
                source: root.backend.readerPageUrl
                onSourceChanged: viewport.contentY = 0
                sourceSize: root.fit === Reader.Height
                    ? Qt.size(0, Math.ceil(viewport.height)) : Qt.size(Math.ceil(viewport.width), 0)
                fillMode: Image.PreserveAspectFit
                asynchronous: true
                visible: status === Image.Ready && !root.backend.readerBusy
                Accessible.role: Accessible.Graphic
                Accessible.name: qsTr("Page %1").arg(root.backend.readerPageIndex + 1)

                TapHandler {
                    acceptedButtons: Qt.LeftButton
                    enabled: page.visible
                    onTapped: eventPoint => {
                        if (eventPoint.position.x < page.width * 0.22)
                            root.backend.turnReaderPage(-1);
                        else if (eventPoint.position.x > page.width * 0.78)
                            root.backend.turnReaderPage(1);
                    }
                }
            }
        }
    }

    BusyIndicator {
        anchors.centerIn: parent
        running: root.backend.readerBusy || page.status === Image.Loading
        visible: running
    }

    ColumnLayout {
        anchors.centerIn: parent
        width: Math.min(480, parent.width - 64)
        visible: root.errorText.length > 0 && !root.backend.readerBusy
        spacing: 16

        Label {
            objectName: "readerError"
            Layout.fillWidth: true
            text: root.errorText
            wrapMode: Text.Wrap
            horizontalAlignment: Text.AlignHCenter
            textFormat: Text.PlainText
        }

        ActionButton {
            objectName: "readerRetry"
            Layout.alignment: Qt.AlignHCenter
            theme: root.theme
            highlighted: true
            text: qsTr("Try again")
            onClicked: root.backend.retryReader()
        }
    }
}

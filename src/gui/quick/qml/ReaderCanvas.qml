// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Qbutt.Content

FocusScope {
    id: root
    objectName: "readerCanvas"
    focus: true

    required property ContentController backend
    required property Theme theme
    readonly property int fit: backend.readerFit
    readonly property string errorText: {
        if (backend.readerErrorText !== "")
            return backend.readerErrorText;

        return page.status === Image.Error ? qsTr("Could not display this page.") : "";
    }

    function saveScroll(): void {
        const range = viewport.contentHeight - viewport.height;
        if (fit !== Reader.Width || !page.visible || page.status !== Image.Ready || range <= 0)
            return;

        backend.scrollReaderPage(page.source, Math.max(0, Math.min(1, viewport.contentY / range)));
    }

    function restoreScroll(offset: real): void {
        if (!page.visible || page.status !== Image.Ready || viewport.moving || scroll.ScrollBar.vertical.pressed)
            return;

        viewport.contentY = offset * Math.max(0, viewport.contentHeight - viewport.height);
    }

    Connections {
        target: root.backend
        function onRestoreReaderScroll(url: url, offset: real): void {
            if (page.source.toString() === url.toString())
                root.restoreScroll(offset);
        }
    }

    Connections {
        target: scroll.ScrollBar.vertical
        function onPressedChanged(): void {
            if (!scroll.ScrollBar.vertical.pressed)
                root.saveScroll();
        }
    }

    ScrollView {
        id: scroll
        objectName: "readerPageScroll"
        anchors.fill: parent
        clip: true
        contentWidth: availableWidth
        visible: root.errorText === ""
        focus: true
        focusPolicy: Qt.StrongFocus
        ScrollBar.vertical.objectName: "readerPageScrollBar"
        Keys.onReleased: event => {
            if (event.key === Qt.Key_Up || event.key === Qt.Key_Down)
                root.saveScroll();
        }

        Flickable {
            id: viewport
            objectName: "readerPageViewport"
            contentWidth: width
            contentHeight: Math.max(height, page.height)
            boundsBehavior: Flickable.StopAtBounds
            flickableDirection: Flickable.VerticalFlick
            onMovementEnded: root.saveScroll()
            onContentHeightChanged: root.restoreScroll(root.backend.readerScrollOffset)
            onHeightChanged: root.restoreScroll(root.backend.readerScrollOffset)

            Image {
                id: page
                objectName: "readerPageImage"
                width: viewport.width
                height: root.fit === Reader.Height ? viewport.height : undefined
                source: root.backend.readerPageUrl
                onSourceChanged: {
                    viewport.cancelFlick();
                    viewport.contentY = 0;
                }
                sourceSize: root.fit === Reader.Height
                    ? Qt.size(0, Math.ceil(viewport.height)) : Qt.size(Math.ceil(viewport.width), 0)
                fillMode: Image.PreserveAspectFit
                asynchronous: true
                visible: status === Image.Ready && !root.backend.readerBusy
                Accessible.role: Accessible.Graphic
                Accessible.name: qsTr("Page %1").arg(root.backend.readerPageIndex + 1)
                onStatusChanged: Qt.callLater(confirmPage)
                onVisibleChanged: Qt.callLater(confirmPage)

                function confirmPage(): void {
                    if (visible && status === Image.Ready) {
                        root.backend.readerPageShown(source);
                        root.restoreScroll(root.backend.readerScrollOffset);
                    }
                }

                TapHandler {
                    acceptedButtons: Qt.LeftButton
                    enabled: page.visible
                    onTapped: eventPoint => {
                        root.forceActiveFocus(Qt.MouseFocusReason);
                        const sideWidth = (page.width - page.paintedWidth) / 2
                            + page.paintedWidth * 0.22;

                        if (eventPoint.position.x < sideWidth) {
                            root.saveScroll();
                            root.backend.turnReaderPage(-1);
                        } else if (eventPoint.position.x > page.width - sideWidth) {
                            root.saveScroll();
                            root.backend.turnReaderPage(1);
                        }
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
        visible: root.errorText !== "" && !root.backend.readerBusy
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

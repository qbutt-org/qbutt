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
    required property color readingForeground
    readonly property int fit: backend.readerFit
    readonly property bool isText: backend.readerIsText
    readonly property bool continuous: backend.readerMode === Reader.Continuous
    readonly property string errorText: backend.readerErrorText
    readonly property real readingProgress: {
        if (isText)
            return textCanvas.status === Loader.Ready
                ? (textCanvas.item as TextReadingCanvas).readingProgress : 0;

        if (continuous)
            return continuousCanvas.status === Loader.Ready
                ? (continuousCanvas.item as ReaderContinuousCanvas).readingProgress : 0;

        if (backend.readerPageCount === 0)
            return 0;

        const visibleFraction = page.visible
            ? Math.min(1, (viewport.contentY + viewport.height) / Math.max(1, page.height)) : 0;
        return (backend.readerPageIndex + visibleFraction) / backend.readerPageCount;
    }

    function saveScroll(): void {
        if (isText) {
            if (textCanvas.status === Loader.Ready)
                (textCanvas.item as TextReadingCanvas).saveScroll();
            return;
        }

        if (continuous) {
            if (continuousCanvas.status === Loader.Ready)
                (continuousCanvas.item as ReaderContinuousCanvas).saveScroll();
            return;
        }

        const range = viewport.contentHeight - viewport.height;
        if (fit !== Reader.Width || !page.visible || page.status !== Image.Ready || range <= 0)
            return;

        backend.scrollReaderPage(page.source, Math.max(0, Math.min(1, viewport.contentY / page.height)));
    }

    function prepareAppearanceChange(): void {
        if (isText && textCanvas.status === Loader.Ready) {
            const text = textCanvas.item as TextReadingCanvas;
            text.preservePosition();
        }
    }

    function restoreScroll(offset: real): void {
        if (!page.visible || page.status !== Image.Ready || viewport.moving || scroll.ScrollBar.vertical.pressed)
            return;

        viewport.contentY = Math.min(offset * page.height,
            Math.max(0, viewport.contentHeight - viewport.height));
    }

    Connections {
        target: root.backend
        function onReaderPositionRequested(generation: real, pageIndex: int, offset: real): void {
            if (!root.isText && !root.continuous && generation === root.backend.readerPages.generation
                && pageIndex === root.backend.readerPageIndex) {
                viewport.cancelFlick();
                root.restoreScroll(offset);
                page.confirmPage();
            }
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
        visible: !root.isText && !root.continuous && root.errorText === ""
        focus: visible
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
                height: root.fit === Reader.Height ? viewport.height
                    : root.backend.readerImageWidth > 0
                        ? Math.ceil(viewport.width * root.backend.readerImageHeight / root.backend.readerImageWidth)
                        : 0
                source: !root.isText && !root.continuous && viewport.width > 0 && viewport.height > 0
                    ? root.backend.readerPageUrl : ""
                onSourceChanged: {
                    viewport.cancelFlick();
                    viewport.contentY = 0;
                }
                sourceSize: root.fit === Reader.Height
                    ? Qt.size(0, Math.min(root.backend.readerImageHeight, Math.ceil(viewport.height)))
                    : Qt.size(Math.min(root.backend.readerImageWidth, Math.ceil(viewport.width)), 0)
                fillMode: Image.PreserveAspectFit
                asynchronous: true
                cache: false
                visible: status === Image.Ready && !root.backend.readerBusy
                Accessible.role: Accessible.Graphic
                Accessible.name: qsTr("Page %1").arg(root.backend.readerPageIndex + 1)
                onStatusChanged: Qt.callLater(confirmPage)
                onVisibleChanged: Qt.callLater(confirmPage)

                function confirmPage(): void {
                    if (status === Image.Error) {
                        root.backend.readerPageFailed(root.backend.readerPageIndex, source);
                        return;
                    }

                    if (visible && status === Image.Ready) {
                        root.backend.readerPageShown(root.backend.readerPageIndex, source);
                        root.restoreScroll(root.backend.readerScrollOffset);
                    }
                }

                TapHandler {
                    acceptedButtons: Qt.LeftButton
                    enabled: page.visible
                    onTapped: root.forceActiveFocus(Qt.MouseFocusReason)
                }
            }
        }
    }

    BusyIndicator {
        anchors.centerIn: parent
        running: root.isText ? root.backend.readerBusy
            : root.continuous ? root.backend.readerBusy && root.backend.readerPageCount === 0
            : root.backend.readerBusy || page.status === Image.Loading
        visible: running
    }

    Loader {
        id: continuousCanvas
        anchors.fill: parent
        active: !root.isText && root.continuous
        visible: root.errorText === ""
        focus: active
        sourceComponent: ReaderContinuousCanvas {
            backend: root.backend
            theme: root.theme
        }
    }

    Loader {
        id: textCanvas
        anchors.fill: parent
        active: root.isText
        visible: root.errorText === "" && !root.backend.readerBusy
        focus: active
        sourceComponent: TextReadingCanvas {
            backend: root.backend
            readingForeground: root.readingForeground
        }
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
            color: root.readingForeground
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

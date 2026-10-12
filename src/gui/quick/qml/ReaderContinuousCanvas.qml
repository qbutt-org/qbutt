// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import Qbutt.Content

ScrollView {
    id: root
    objectName: "readerContinuousScroll"

    required property ContentController backend
    required property Theme theme
    property var pendingPosition: null
    property bool userScrolling: false

    contentWidth: availableWidth
    clip: true
    focus: true
    focusPolicy: Qt.StrongFocus
    ScrollBar.vertical.objectName: "readerContinuousScrollBar"
    ScrollBar.vertical.stepSize: 40 / Math.max(40, pageList.contentHeight)

    function requestPosition(generation: real, pageIndex: int, offset: real): void {
        if (parent === null || backend.readerMode !== Reader.Continuous
            || generation !== backend.readerPages.generation)
            return;

        userScrolling = false;
        pendingPosition = {
            generation: generation,
            pageIndex: pageIndex,
            offset: offset
        };
        Qt.callLater(restorePosition);
    }

    function retainPosition(): void {
        if (parent !== null && backend.readerMode === Reader.Continuous && pendingPosition === null)
            requestPosition(backend.readerPages.generation, backend.readerPageIndex, backend.readerScrollOffset);
    }

    function restorePosition(): void {
        if (parent === null || backend.readerMode !== Reader.Continuous || pendingPosition === null
            || pageList.count === 0 || pageList.width <= 0 || pageList.height <= 0)
            return;

        if (pendingPosition.generation !== backend.readerPages.generation) {
            pendingPosition = null;
            return;
        }

        const position = pendingPosition;
        pageList.forceLayout();
        pageList.positionViewAtIndex(position.pageIndex, ListView.Beginning);
        pageList.forceLayout();
        const page = pageList.itemAtIndex(position.pageIndex) as ReaderStripPage;
        if (!page)
            return;

        pageList.contentY = page.y + position.offset * page.height;
        pageList.returnToBounds();
        pendingPosition = null;
        page.confirmPage();
        Qt.callLater(saveScroll);
    }

    function saveScroll(): void {
        if (parent === null || backend.readerMode !== Reader.Continuous || pendingPosition !== null
            || pageList.count === 0 || pageList.width <= 0 || pageList.height <= 0)
            return;

        const first = pageList.itemAt(pageList.width / 2, pageList.contentY + 0.5) as ReaderStripPage;
        if (!first)
            return;

        const bottomIndex = pageList.indexAt(pageList.width / 2, pageList.contentY + pageList.height - 0.5);
        const last = bottomIndex < 0 ? pageList.count - 1 : bottomIndex;
        const offset = Math.max(0, Math.min(1, (pageList.contentY - first.y) / first.height));
        const userMoved = userScrolling;
        if (!pageList.moving && !root.ScrollBar.vertical.pressed && !wheelObserver.active)
            userScrolling = false;
        backend.setReaderViewport(backend.readerPages.generation,
            first.pageIndex, last, first.pageIndex, offset, userMoved);
    }

    Component.onCompleted: requestPosition(backend.readerPages.generation, backend.readerPageIndex, backend.readerScrollOffset)

    Connections {
        target: root.backend
        enabled: root.parent !== null && root.backend.readerMode === Reader.Continuous

        function onReaderPositionRequested(generation: real, pageIndex: int, offset: real): void {
            root.requestPosition(generation, pageIndex, offset);
            pageList.cancelFlick();
        }
    }

    Connections {
        target: root.ScrollBar.vertical

        function onPressedChanged(): void {
            if (root.ScrollBar.vertical.pressed)
                root.userScrolling = true;
            else {
                root.saveScroll();
                root.userScrolling = false;
            }
        }
    }

    Keys.onPressed: event => {
        if (event.key === Qt.Key_Up || event.key === Qt.Key_Down) {
            root.userScrolling = true;
            return;
        }

        if (event.key !== Qt.Key_PageUp && event.key !== Qt.Key_PageDown)
            return;

        const direction = event.key === Qt.Key_PageDown ? 1 : -1;
        const maximum = Math.max(pageList.originY, pageList.originY + pageList.contentHeight - pageList.height);
        pageList.cancelFlick();
        root.userScrolling = true;
        pageList.contentY = Math.max(pageList.originY,
            Math.min(maximum, pageList.contentY + direction * pageList.height));
        event.accepted = true;
    }

    contentItem: ListView {
        id: pageList
        objectName: "readerContinuousViewport"
        model: root.backend.readerPages
        currentIndex: -1
        keyNavigationEnabled: false
        cacheBuffer: height
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.VerticalFlick
        onContentYChanged: Qt.callLater(root.saveScroll)
        onDraggingChanged: {
            if (dragging)
                root.userScrolling = true;
        }
        onMovementEnded: {
            root.saveScroll();
            root.userScrolling = false;
        }
        onCountChanged: Qt.callLater(root.restorePosition)
        onWidthChanged: root.retainPosition()
        onHeightChanged: root.retainPosition()

        delegate: ReaderStripPage {
            backend: root.backend
            theme: root.theme
            viewport: pageList
            onGeometryChanged: root.retainPosition()
        }

        WheelHandler {
            id: wheelObserver
            target: null
            blocking: false
            acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
            onWheel: root.userScrolling = true
            onActiveChanged: {
                if (!active)
                    root.saveScroll();
            }
        }
    }
}

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
    readonly property real readingProgress: pageList.count === 0 ? 0
        : Math.max(0, Math.min(1,
            (pageList.contentY - pageList.originY + pageList.height)
                / Math.max(1, pageList.contentHeight)))
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
        if (parent !== null && backend.readerMode === Reader.Continuous && pendingPosition === null
            && !userScrolling && !pageList.moving)
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
        const userMoved = userScrolling && first.imageReady;
        if (first.imageReady && !pageList.moving && !root.ScrollBar.vertical.pressed && !wheelObserver.active)
            userScrolling = false;
        backend.setReaderViewport(backend.readerPages.generation,
            first.pageIndex, last, first.pageIndex, offset, userMoved);
    }

    function scrollPage(direction: int): void {
        const maximum = Math.max(pageList.originY, pageList.originY + pageList.contentHeight - pageList.height);
        pageList.cancelFlick();
        const position = Math.max(pageList.originY,
            Math.min(maximum, pageList.contentY + direction * pageList.height));
        if (position === pageList.contentY)
            return;

        userScrolling = true;
        pageList.contentY = position;
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
            else
                root.saveScroll();
        }
    }

    Keys.onPressed: event => {
        if ((event.key === Qt.Key_Up && !pageList.atYBeginning)
            || (event.key === Qt.Key_Down && !pageList.atYEnd))
            root.userScrolling = true;
    }

    Shortcut {
        sequence: "PgUp"
        enabled: root.visible && root.activeFocus
        onActivated: root.scrollPage(-1)
    }

    Shortcut {
        sequence: "PgDown"
        enabled: root.visible && root.activeFocus
        onActivated: root.scrollPage(1)
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
        onMovementEnded: root.saveScroll()
        onCountChanged: Qt.callLater(root.restorePosition)
        onWidthChanged: root.retainPosition()
        onHeightChanged: root.retainPosition()

        delegate: ReaderStripPage {
            backend: root.backend
            theme: root.theme
            viewport: pageList
            onGeometryChanged: root.retainPosition()
            onImageReadyChanged: {
                if (imageReady)
                    Qt.callLater(root.saveScroll);
            }
        }

        WheelHandler {
            id: wheelObserver
            target: null
            blocking: false
            acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
            onWheel: event => {
                const delta = event.pixelDelta.y || event.angleDelta.y;
                if ((delta > 0 && !pageList.atYBeginning) || (delta < 0 && !pageList.atYEnd))
                    root.userScrolling = true;
            }
            onActiveChanged: {
                if (!active)
                    root.saveScroll();
            }
        }
    }
}

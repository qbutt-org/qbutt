// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import Qbutt.Content

ScrollView {
    id: root
    objectName: "readerTextScroll"

    required property ContentController backend
    required property Theme theme
    property var pendingPosition: null
    property bool userScrolling: false

    contentWidth: availableWidth
    clip: true
    focus: true
    focusPolicy: Qt.StrongFocus
    ScrollBar.vertical.objectName: "readerTextScrollBar"
    ScrollBar.vertical.stepSize: 40 / Math.max(40, blocks.contentHeight)

    function requestPosition(generation: real, blockId: string, offset: real): void {
        if (generation !== backend.readerTextGeneration)
            return;

        userScrolling = false;
        pendingPosition = { generation: generation, blockId: blockId, offset: offset };
        blocks.cancelFlick();
        Qt.callLater(restorePosition);
    }

    function restorePosition(): void {
        if (pendingPosition === null || backend.readerBusy || blocks.count === 0
            || blocks.width <= 0 || blocks.height <= 0)
            return;

        const position = pendingPosition;
        if (position.generation !== backend.readerTextGeneration) {
            pendingPosition = null;
            return;
        }

        const index = backend.readerTextBlocks.indexOfId(position.blockId);
        if (index < 0)
            return;

        blocks.forceLayout();
        blocks.positionViewAtIndex(index, ListView.Beginning);
        blocks.forceLayout();
        const block = blocks.itemAtIndex(index);
        if (!block)
            return;

        blocks.contentY = block.y + position.offset * block.height;
        blocks.returnToBounds();
        pendingPosition = null;
        Qt.callLater(saveScroll);
    }

    function saveScroll(): void {
        if (!visible || pendingPosition !== null || backend.readerBusy
            || backend.readerErrorText !== "" || blocks.count === 0)
            return;

        const index = blocks.indexAt(blocks.width / 2, blocks.contentY + 0.5);
        const first = blocks.itemAtIndex(index);
        if (!first)
            return;

        const offset = Math.max(0, Math.min(1, (blocks.contentY - first.y) / first.height));
        const userMoved = userScrolling;
        if (!blocks.moving && !root.ScrollBar.vertical.pressed && !wheelObserver.active)
            userScrolling = false;
        backend.setReaderTextViewport(backend.readerTextGeneration,
            backend.readerTextBlocks.idAt(index), offset, userMoved);
    }

    function retainPosition(): void {
        if (pendingPosition === null && !userScrolling && !blocks.moving)
            requestPosition(backend.readerTextGeneration, backend.readerTextAnchor, backend.readerTextOffset);
    }

    function scrollBy(distance: real): void {
        const maximum = Math.max(blocks.originY, blocks.originY + blocks.contentHeight - blocks.height);
        const position = Math.max(blocks.originY, Math.min(maximum, blocks.contentY + distance));
        if (position === blocks.contentY)
            return;

        blocks.cancelFlick();
        userScrolling = true;
        blocks.contentY = position;
    }

    Component.onCompleted: requestPosition(backend.readerTextGeneration, backend.readerTextAnchor, backend.readerTextOffset)

    Connections {
        target: root.backend

        function onReaderTextPositionRequested(generation: real, blockId: string, offset: real): void {
            root.requestPosition(generation, blockId, offset);
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
        switch (event.key) {
        case Qt.Key_Up:
            root.scrollBy(-40);
            break;
        case Qt.Key_Down:
            root.scrollBy(40);
            break;
        case Qt.Key_PageUp:
            root.scrollBy(-blocks.height * 0.9);
            break;
        case Qt.Key_PageDown:
            root.scrollBy(blocks.height * 0.9);
            break;
        case Qt.Key_Home:
            if (!blocks.atYBeginning) {
                root.userScrolling = true;
                blocks.positionViewAtBeginning();
            }
            break;
        case Qt.Key_End:
            if (!blocks.atYEnd) {
                root.userScrolling = true;
                blocks.positionViewAtEnd();
            }
            break;
        default:
            return;
        }
        event.accepted = true;
    }

    contentItem: ListView {
        id: blocks
        objectName: "readerTextViewport"
        model: root.backend.readerTextBlocks
        currentIndex: -1
        keyNavigationEnabled: false
        reuseItems: true
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

        delegate: Column {
            id: readingBlock
            required property string blockId
            required property int blockKind
            required property string plainText
            required property string richText
            required property string imageId
            required property int listOrdinal

            objectName: "readerTextBlock_%1".arg(blockId)
            width: ListView.view.width
            height: implicitHeight + 18

            Label {
                visible: readingBlock.blockKind !== 5
                width: readingBlock.width
                leftPadding: Math.max(0, (width - 760) / 2) + (readingBlock.blockKind === 3 || readingBlock.blockKind === 4 ? 24 : 0)
                rightPadding: Math.max(0, (width - 760) / 2)
                topPadding: readingBlock.blockKind === 1 ? 16 : 0
                text: readingBlock.blockKind === 3
                    ? (readingBlock.listOrdinal >= 0 ? "%1. ".arg(readingBlock.listOrdinal) : "– ") + readingBlock.richText
                    : readingBlock.richText
                textFormat: Text.StyledText
                wrapMode: Text.Wrap
                color: root.theme.foreground
                font.pixelSize: readingBlock.blockKind === 1 ? 24 : 20
                font.weight: readingBlock.blockKind === 1 ? Font.DemiBold : Font.Normal
                font.italic: readingBlock.blockKind === 4
                lineHeight: 1.3
                horizontalAlignment: readingBlock.blockKind === 2 ? Text.AlignHCenter : Text.AlignLeft
                Accessible.role: Accessible.StaticText
                Accessible.name: readingBlock.plainText
            }

            Image {
                visible: readingBlock.blockKind === 5
                x: Math.max(0, (readingBlock.width - 760) / 2)
                width: Math.min(760, readingBlock.width)
                height: status === Image.Ready && implicitWidth > 0
                    ? width * implicitHeight / implicitWidth : 0
                source: visible && readingBlock.imageId !== ""
                    ? "image://reader/%1/%2".arg(root.backend.readerTextGeneration).arg(readingBlock.imageId) : ""
                asynchronous: true
                cache: false
                fillMode: Image.PreserveAspectFit
                Accessible.role: Accessible.Graphic
                Accessible.name: readingBlock.plainText
            }

            onHeightChanged: Qt.callLater(root.retainPosition)
        }

        TapHandler {
            onTapped: root.forceActiveFocus(Qt.MouseFocusReason)
        }

        WheelHandler {
            id: wheelObserver
            target: null
            blocking: false
            acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
            onWheel: event => {
                const delta = event.pixelDelta.y || event.angleDelta.y;
                if ((delta > 0 && !blocks.atYBeginning) || (delta < 0 && !blocks.atYEnd))
                    root.userScrolling = true;
            }
            onActiveChanged: {
                if (!active)
                    root.saveScroll();
            }
        }
    }
}

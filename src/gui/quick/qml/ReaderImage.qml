// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick

Item {
    id: root

    required property url source
    required property int imageWidth
    required property int imageHeight
    required property size sourceSize
    required property real viewportTop
    required property real viewportHeight
    required property bool tilingEnabled
    property int status: Image.Null

    readonly property bool hasSource: source.toString() !== "" && imageWidth > 0 && imageHeight > 0
    readonly property bool tiled: tilingEnabled && imageHeight > 16000
    readonly property real displayScale: hasSource
        ? Math.min(width / imageWidth, height / imageHeight) : 0
    readonly property real paintedWidth: imageWidth * displayScale
    readonly property real paintedHeight: imageHeight * displayScale
    readonly property real imageLeft: (width - paintedWidth) / 2
    readonly property real imageTop: (height - paintedHeight) / 2
    readonly property int tileHeight: Math.min(2048, Math.max(1, Math.floor(2000000 / Math.max(1, imageWidth))))

    onSourceChanged: {
        status = hasSource ? Image.Loading : Image.Null;
        Qt.callLater(updateStatus);
    }
    Component.onCompleted: Qt.callLater(updateStatus)

    function updateStatus(): void {
        if (!hasSource) {
            status = Image.Null;
            return;
        }
        if (!tiled) {
            const image = overview.status === Loader.Ready && overview.item !== null
                ? overview.item as Image : null;
            status = image !== null && image.source === source ? image.status : Image.Loading;
            return;
        }
        let found = false;
        let loading = false;
        for (let index = 0; index < tiles.count; ++index) {
            const tile = tiles.itemAt(index) as TileLoader;
            if (tile === null || !tile.active)
                continue;

            found = true;
            if (tile.status !== Loader.Ready || tile.item === null) {
                loading = true;
                continue;
            }
            const image = tile.item as Image;
            if (image.source !== tile.tileSource) {
                loading = true;
                continue;
            }
            if (image.status === Image.Error) {
                status = Image.Error;
                return;
            }
            loading = loading || image.status !== Image.Ready;
        }
        status = !found ? Image.Null : loading ? Image.Loading : Image.Ready;
    }

    Loader {
        id: overview
        anchors.fill: parent
        active: root.hasSource && !root.tiled
        onStatusChanged: root.updateStatus()

        sourceComponent: Image {
            anchors.fill: parent
            source: root.source
            sourceSize: root.sourceSize
            fillMode: Image.PreserveAspectFit
            asynchronous: true
            cache: false
            visible: status === Image.Ready
            Accessible.ignored: true
            onStatusChanged: root.updateStatus()
        }
    }

    component TileLoader: Loader {
        id: tile
        required property int index
        readonly property int sourceTop: index * root.tileHeight
        readonly property int sourceHeight: Math.min(root.tileHeight, root.imageHeight - sourceTop)
        readonly property url tileSource: "image://reader/tile/"
            + encodeURIComponent(root.source.toString()) + "/" + sourceTop + "/" + sourceHeight

        objectName: "readerTileLoader%1".arg(index)
        x: root.imageLeft
        y: root.imageTop + sourceTop * root.displayScale
        width: root.paintedWidth
        height: sourceHeight * root.displayScale
        active: y + height > root.viewportTop && y < root.viewportTop + root.viewportHeight
        onActiveChanged: root.updateStatus()
        onStatusChanged: root.updateStatus()

        sourceComponent: Image {
            objectName: "readerTileImage%1".arg(tile.index)
            anchors.fill: parent
            source: tile.tileSource
            sourceSize: Qt.size(root.imageWidth, tile.sourceHeight)
            fillMode: Image.PreserveAspectFit
            asynchronous: true
            cache: false
            visible: status === Image.Ready
            Accessible.ignored: true
            onStatusChanged: root.updateStatus()
        }
    }

    Repeater {
        id: tiles
        model: root.hasSource && root.tiled ? Math.ceil(root.imageHeight / root.tileHeight) : 0
        delegate: TileLoader {}
    }
}

// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Qbutt.Content

Item {
    id: root

    required property ContentController backend
    required property Theme theme
    required property ListView viewport
    required property int pageIndex
    required property url pageUrl
    required property int imageWidth
    required property int imageHeight
    required property int pageState
    required property string errorText
    signal geometryChanged

    readonly property bool hasDimensions: imageWidth > 0 && imageHeight > 0
    readonly property real displayScale: hasDimensions
        ? (backend.readerFit === Reader.Height
            ? Math.min(viewport.width / imageWidth, viewport.height / imageHeight)
            : viewport.width / imageWidth)
        : 0
    readonly property bool inViewport: y + height > viewport.contentY
        && y < viewport.contentY + viewport.height
    readonly property real visibleTop: Math.max(0, viewport.contentY - y)
    readonly property real visibleHeight: Math.max(0,
        Math.min(height, viewport.contentY + viewport.height - y) - visibleTop)

    objectName: "readerStripPage%1".arg(pageIndex)
    width: viewport.width
    height: hasDimensions ? Math.ceil(imageHeight * displayScale) : viewport.height
    onHeightChanged: geometryChanged()

    function confirmPage(): void {
        if (!inViewport || imageLoader.status !== Loader.Ready || imageLoader.item === null)
            return;

        const page = imageLoader.item as Image;
        if (page.visible && page.status === Image.Ready)
            backend.readerPageShown(pageIndex, page.source);
    }

    Loader {
        id: imageLoader
        anchors.horizontalCenter: parent.horizontalCenter
        width: root.hasDimensions ? Math.ceil(root.imageWidth * root.displayScale) : 0
        height: root.height
        active: root.inViewport && root.pageState === Reader.PageReady

        sourceComponent: Image {
            id: page
            objectName: "readerStripImage%1".arg(root.pageIndex)
            width: imageLoader.width
            height: imageLoader.height
            source: root.pageUrl
            sourceSize: Qt.size(
                Math.min(root.imageWidth, Math.ceil(width)),
                Math.min(root.imageHeight, Math.ceil(height)))
            fillMode: Image.PreserveAspectFit
            asynchronous: true
            cache: false
            visible: status === Image.Ready
            Accessible.role: Accessible.Graphic
            Accessible.name: qsTranslate("ReaderCanvas", "Page %1").arg(root.pageIndex + 1)
            onStatusChanged: {
                if (status === Image.Ready)
                    Qt.callLater(root.confirmPage);
                else if (status === Image.Error)
                    root.backend.readerPageFailed(root.pageIndex, source);
            }
        }
    }

    BusyIndicator {
        anchors.horizontalCenter: parent.horizontalCenter
        y: root.visibleTop + (root.visibleHeight - height) / 2
        running: root.inViewport && root.pageState !== Reader.PageFailed
            && (imageLoader.status !== Loader.Ready
                || imageLoader.item === null
                || (imageLoader.item as Image).status !== Image.Ready)
        visible: running
    }

    ColumnLayout {
        anchors.horizontalCenter: parent.horizontalCenter
        y: root.visibleTop + (root.visibleHeight - height) / 2
        width: Math.min(480, root.width - 64)
        visible: root.inViewport && root.pageState === Reader.PageFailed
        spacing: 16

        Label {
            Layout.fillWidth: true
            text: root.errorText
            textFormat: Text.PlainText
            wrapMode: Text.Wrap
            horizontalAlignment: Text.AlignHCenter
        }

        ActionButton {
            objectName: "readerStripRetry%1".arg(root.pageIndex)
            Layout.alignment: Qt.AlignHCenter
            theme: root.theme
            highlighted: true
            text: qsTranslate("ReaderCanvas", "Try again")
            onClicked: root.backend.retryReaderPage(root.backend.readerPages.generation, root.pageIndex)
        }
    }
}

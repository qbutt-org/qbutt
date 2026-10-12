// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Qbutt.Content

Page {
    id: root
    objectName: "contentDownloadsPage"

    required property ContentController backend
    signal openFolderRequested(string path)

    implicitWidth: 900
    implicitHeight: 560
    padding: 24
    palette.window: applicationPalette.window
    palette.windowText: applicationPalette.windowText
    palette.highlight: appearance.accent

    SystemPalette {
        id: applicationPalette
        colorGroup: SystemPalette.Active
    }

    Theme {
        id: appearance
        background: root.palette.window
        foreground: root.palette.windowText
    }

    function sizeText(bytes: real): string {
        if (bytes < 1024)
            return qsTr("%1 B").arg(Math.round(bytes));
        if (bytes < 1024 * 1024)
            return qsTr("%1 KB").arg((bytes / 1024).toFixed(1));
        if (bytes < 1024 * 1024 * 1024)
            return qsTr("%1 MB").arg((bytes / (1024 * 1024)).toFixed(1));
        return qsTr("%1 GB").arg((bytes / (1024 * 1024 * 1024)).toFixed(1));
    }

    contentItem: ColumnLayout {
        spacing: 20

        Label {
            Layout.fillWidth: true
            text: qsTr("Downloads")
            font.pixelSize: 24
            font.weight: Font.DemiBold
        }

        ListView {
            id: jobs
            objectName: "contentDownloads"
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: root.backend.downloads
            spacing: 8
            reuseItems: true
            ScrollBar.vertical: ScrollBar {}

            delegate: Item {
                id: row
                required property string jobId
                required property string title
                required property string status
                required property real bytes
                required property var totalBytes
                required property string path
                required property string errorText
                required property bool canRetry

                width: ListView.view.width
                height: Math.max(76, details.implicitHeight + 16)

                RowLayout {
                    id: details
                    anchors.fill: parent
                    anchors.leftMargin: 4
                    anchors.rightMargin: 4
                    spacing: 16

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 5

                        Label {
                            Layout.fillWidth: true
                            text: row.title
                            font.weight: Font.DemiBold
                            elide: Text.ElideRight
                            textFormat: Text.PlainText
                        }

                        Label {
                            Layout.fillWidth: true
                            color: appearance.muted
                            text: row.status === "queued" ? qsTr("Queued")
                                : row.status === "downloading" ? (row.totalBytes > 0
                                    ? qsTr("%1 of %2").arg(root.sizeText(row.bytes)).arg(root.sizeText(row.totalBytes))
                                    : qsTr("Downloading %1").arg(root.sizeText(row.bytes)))
                                : row.status === "publishing" ? qsTr("Saving file")
                                : row.status === "complete" ? qsTr("Complete")
                                : row.status === "cancelled" ? qsTr("Stopped")
                                : row.errorText || qsTr("Failed")
                            elide: Text.ElideRight
                            textFormat: Text.PlainText
                        }

                        Label {
                            Layout.fillWidth: true
                            visible: !row.canRetry && (row.status === "cancelled" || row.status === "error")
                            color: appearance.muted
                            text: qsTr("Choose the video again in qcatch")
                            elide: Text.ElideRight
                        }

                        ProgressBar {
                            Layout.fillWidth: true
                            Layout.preferredHeight: 3
                            visible: row.status === "downloading" || row.status === "publishing"
                            indeterminate: row.totalBytes <= 0 || row.status === "publishing"
                            value: row.totalBytes > 0 ? Math.min(1, row.bytes / row.totalBytes) : 0
                        }
                    }

                    IconButton {
                        objectName: "downloadOpenFolder"
                        theme: appearance
                        visible: row.status === "complete" && row.path.length > 0
                        icon.source: appearance.iconSource("folder-open")
                        text: qsTr("Open folder")
                        onClicked: root.openFolderRequested(row.path)
                    }

                    IconButton {
                        objectName: "downloadCancel"
                        theme: appearance
                        visible: row.status === "queued" || row.status === "downloading"
                        icon.source: appearance.iconSource("close")
                        text: qsTr("Stop download")
                        onClicked: root.backend.cancelDownload(row.jobId)
                    }

                    IconButton {
                        objectName: "downloadRetry"
                        theme: appearance
                        visible: row.canRetry && (row.status === "cancelled" || row.status === "error")
                        icon.source: appearance.iconSource("view-refresh")
                        text: qsTr("Retry")
                        onClicked: root.backend.retryDownload(row.jobId)
                    }
                }
            }

            Label {
                anchors.centerIn: parent
                visible: jobs.count === 0 && root.backend.downloadsState !== "loading"
                text: root.backend.downloadsState === "failed"
                    ? qsTr("Could not open downloads") : qsTr("No downloads yet")
                color: appearance.muted
            }

            BusyIndicator {
                anchors.centerIn: parent
                visible: jobs.count === 0 && root.backend.downloadsState === "loading"
                running: visible
            }
        }
    }
}

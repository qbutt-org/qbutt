// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Qbutt.Content

Page {
    id: root
    objectName: "contentQuickSearch"

    required property ContentController backend

    implicitWidth: 1200
    implicitHeight: 560
    padding: 24
    palette.highlight: appearance.accent

    Theme {
        id: appearance
        background: root.palette.window
        foreground: root.palette.windowText
    }

    Shortcut {
        sequences: [StandardKey.Find]
        enabled: root.visible
        onActivated: {
            query.forceActiveFocus();
            query.selectAll();
        }
    }

    function submitSearch(): void {
        const text = query.text.trim();
        if (text.length === 0)
            return;

        backend.search(text);
    }

    background: Rectangle {
        color: appearance.background
    }

    contentItem: ColumnLayout {
        spacing: 16

        RowLayout {
            spacing: 12

            TextField {
                id: query
                objectName: "contentQuery"
                Layout.fillWidth: true
                Layout.preferredHeight: appearance.controlHeight
                placeholderText: qsTr("Search anime")
                selectByMouse: true
                Accessible.name: qsTr("Search anime")
                onAccepted: root.submitSearch()
            }

            Button {
                objectName: "contentSearch"
                Layout.preferredHeight: appearance.controlHeight
                text: root.backend.busy ? qsTr("Stop") : qsTr("Search")
                enabled: root.backend.busy || query.text.trim().length > 0
                onClicked: {
                    if (root.backend.busy)
                        root.backend.cancelSearch();
                    else
                        root.submitSearch();
                }
            }
        }

        ProgressBar {
            Layout.fillWidth: true
            Layout.preferredHeight: 3
            visible: root.backend.busy
            indeterminate: true
        }

        Label {
            objectName: "contentError"
            Layout.fillWidth: true
            visible: root.backend.errorText !== ""
            text: root.backend.errorText
            wrapMode: Text.Wrap
            textFormat: Text.PlainText
        }

        SplitView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            handle: Rectangle {
                implicitWidth: 8
                color: SplitHandle.hovered || SplitHandle.pressed ? appearance.hover : "transparent"
            }

            Item {
                SplitView.minimumWidth: 280
                SplitView.preferredWidth: 340
                SplitView.maximumWidth: 440

                ListView {
                    id: works
                    objectName: "contentResults"
                    anchors.fill: parent
                    anchors.rightMargin: 16
                    clip: true
                    model: root.backend.works
                    currentIndex: -1
                    spacing: 4
                    activeFocusOnTab: true
                    keyNavigationEnabled: true
                    reuseItems: true
                    ScrollBar.vertical: ScrollBar {}

                    delegate: WorkDelegate {
                        id: work
                        theme: appearance
                        highlighted: root.backend.selectedWorkId === work.workId
                        onClicked: {
                            works.currentIndex = work.index;
                            works.forceActiveFocus();
                            root.backend.selectWork(work.workId);
                        }
                    }

                    function selectCurrentWork(): void {
                        const selected = currentItem as WorkDelegate;
                        if (activeFocus && selected)
                            root.backend.selectWork(selected.workId);
                    }

                    Keys.onReturnPressed: selectCurrentWork()
                    Keys.onEnterPressed: selectCurrentWork()
                    Keys.onPressed: event => {
                        if (count > 0 && (event.key === Qt.Key_Home || event.key === Qt.Key_End)) {
                            currentIndex = event.key === Qt.Key_Home ? 0 : count - 1;
                            positionViewAtIndex(currentIndex, event.key === Qt.Key_Home ? ListView.Beginning : ListView.End);
                            event.accepted = true;
                        }
                    }
                    Keys.onReleased: event => {
                        if (event.key === Qt.Key_Up || event.key === Qt.Key_Down || event.key === Qt.Key_Home || event.key === Qt.Key_End) {
                            selectCurrentWork();
                        }
                    }
                }

                Label {
                    objectName: "contentEmptyResults"
                    anchors.centerIn: parent
                    width: parent.width - 32
                    visible: root.backend.searchState === Search.Succeeded && works.count === 0
                    text: qsTr("No titles found")
                    color: appearance.muted
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                }
            }

            WorkPane {
                SplitView.fillWidth: true
                SplitView.minimumWidth: 560
                backend: root.backend
                theme: appearance
                leftPadding: 24
            }
        }
    }
}

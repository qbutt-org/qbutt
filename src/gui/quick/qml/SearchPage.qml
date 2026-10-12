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
    readonly property WorkModel activeWorks: backend.listMode === Search.Favorites ? backend.favorites : backend.works

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
        enabled: root.visible && !root.backend.readerOpen
        onActivated: {
            root.backend.setListMode(Search.Search);
            query.forceActiveFocus();
            query.selectAll();
        }
    }

    function submitSearch(): void {
        const text = query.text.trim();
        if (text.length === 0)
            return;

        backend.setListMode(Search.Search);
        backend.search(text);
    }

    background: Rectangle {
        color: appearance.background
    }

    contentItem: ColumnLayout {
        visible: !root.backend.readerOpen
        spacing: 16

        RowLayout {
            spacing: 12

            TabBar {
                objectName: "contentModes"
                Layout.preferredWidth: searchMode.implicitWidth + favoritesMode.implicitWidth + spacing
                spacing: 8
                currentIndex: root.backend.listMode === Search.Favorites ? 1 : 0
                onCurrentIndexChanged: root.backend.setListMode(currentIndex === 1 ? Search.Favorites : Search.Search)

                background: Item {}

                NavigationTab {
                    id: searchMode
                    objectName: "contentSearchMode"
                    theme: appearance
                    text: qsTr("Search")
                }

                NavigationTab {
                    id: favoritesMode
                    objectName: "contentFavoritesMode"
                    theme: appearance
                    text: qsTr("Favorites")
                }
            }

            QueryField {
                id: query
                objectName: "contentQuery"
                Layout.fillWidth: true
                theme: appearance
                placeholderText: qsTr("Search titles")
                selectByMouse: true
                Accessible.name: qsTr("Search titles")
                onAccepted: root.submitSearch()
            }

            ActionButton {
                objectName: "contentSearch"
                theme: appearance
                highlighted: !root.backend.busy
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

        RowLayout {
            Layout.fillWidth: true
            visible: root.backend.libraryErrorText !== ""
            spacing: 16

            Label {
                objectName: "contentLibraryError"
                Layout.fillWidth: true
                text: root.backend.libraryErrorText
                wrapMode: Text.Wrap
                textFormat: Text.PlainText
            }

            ActionButton {
                objectName: "contentLibraryRetry"
                theme: appearance
                text: qsTr("Try again")
                visible: root.backend.libraryState === Library.Failed
                onClicked: root.backend.retryLibrary()
            }
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
                    model: root.activeWorks
                    currentIndex: -1
                    onCountChanged: restoreSelection()
                    spacing: 4
                    activeFocusOnTab: true
                    keyNavigationEnabled: true
                    ScrollBar.vertical: ScrollBar {}

                    function restoreSelection(): void {
                        currentIndex = root.activeWorks.indexOfId(root.backend.selectedWorkId);
                    }

                    Connections {
                        target: root.backend
                        function onSelectedWorkIdChanged(): void {
                            works.restoreSelection();
                        }
                    }

                    delegate: WorkDelegate {
                        id: work
                        theme: appearance
                        highlighted: root.backend.selectedWorkId === work.workId
                        onClicked: {
                            works.forceActiveFocus();
                            root.backend.selectWork(work.workId);
                        }
                    }

                    function selectCurrentWork(): void {
                        if (activeFocus && currentIndex >= 0)
                            root.backend.selectWork(root.activeWorks.idAt(currentIndex));
                    }

                    Keys.onReturnPressed: selectCurrentWork()
                    Keys.onEnterPressed: selectCurrentWork()
                    Keys.onPressed: event => {
                        if (count > 0 && (event.key === Qt.Key_Home || event.key === Qt.Key_End)) {
                            const index = event.key === Qt.Key_Home ? 0 : count - 1;
                            root.backend.selectWork(root.activeWorks.idAt(index));
                            positionViewAtIndex(index, event.key === Qt.Key_Home ? ListView.Beginning : ListView.End);
                            event.accepted = true;
                        }
                    }
                    Keys.onReleased: event => {
                        if (event.key === Qt.Key_Up || event.key === Qt.Key_Down) {
                            selectCurrentWork();
                        }
                    }
                }

                Label {
                    objectName: "contentEmptyResults"
                    anchors.centerIn: parent
                    width: parent.width - 32
                    visible: works.count === 0 && (root.backend.listMode === Search.Favorites
                        ? root.backend.libraryState === Library.Ready : root.backend.searchState === Search.Succeeded)
                    text: root.backend.listMode === Search.Favorites ? qsTr("No favorites") : qsTr("No titles found")
                    color: appearance.muted
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                }

                BusyIndicator {
                    anchors.centerIn: parent
                    visible: root.backend.listMode === Search.Favorites
                        && (root.backend.libraryState === Library.Uninitialized || root.backend.libraryState === Library.Loading)
                    running: visible
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

    Loader {
        id: reader
        parent: root
        anchors.fill: parent
        active: root.backend.readerOpen
        focus: active
        onActiveChanged: {
            if (!active)
                query.forceActiveFocus();
        }

        sourceComponent: ReaderPage {
            backend: root.backend
            theme: appearance
        }
    }
}

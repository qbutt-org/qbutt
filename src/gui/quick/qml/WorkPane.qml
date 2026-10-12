// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Qbutt.Content

Pane {
    id: root
    objectName: "contentWorkPane"

    required property ContentController backend
    required property Theme theme
    required property Item backdrop
    required property bool selectedInList
    padding: 0

    readonly property int sourceWidth: 116
    readonly property int sizeWidth: 80
    readonly property int seedersWidth: 48
    readonly property bool showTorrentColumns: filteredVariants.hasTorrents
    readonly property bool selectedForReading: backend.selectedVariantKind === "manga"
        || backend.selectedVariantKind === "text"
    readonly property int selectedRow: {
        return filteredVariants.count > 0 ? filteredVariants.indexOfId(backend.selectedVariantId) : -1;
    }

    VariantFilterModel {
        id: filteredVariants
        objectName: "contentFilteredVariants"
        sourceModel: root.backend.variants
        episodeFilter: root.backend.variantEpisodeFilter
        translationFilter: root.backend.variantTranslationFilter
        sourceFilter: root.backend.variantSourceFilter
        playerFilter: root.backend.variantPlayerFilter
        languageFilter: root.backend.variantLanguageFilter
        transportFilter: root.backend.variantTransportFilter
        query: root.backend.variantQuery
    }

    function openSelected(): void {
        if (backend.selectedVariantState !== Search.Available || backend.resolving || selectedRow < 0)
            return;

        if (selectedForReading)
            backend.openReader();
        else
            backend.resolveVariant(backend.selectedVariantId);
    }

    background: Item {}

    contentItem: ColumnLayout {
        visible: root.selectedInList
        spacing: 16

        RowLayout {
            Layout.fillWidth: false
            Layout.maximumWidth: root.availableWidth
            Layout.alignment: Qt.AlignLeft
            spacing: 12

            Label {
                objectName: "contentTitle"
                Layout.fillWidth: true
                text: root.backend.selectedWorkTitle
                font.pixelSize: 24
                font.weight: Font.DemiBold
                wrapMode: Text.Wrap
                maximumLineCount: 3
                elide: Text.ElideRight
                textFormat: Text.PlainText
            }

            IconButton {
                objectName: "contentFavorite"
                theme: root.theme
                Layout.alignment: Qt.AlignTop
                icon.source: root.backend.selectedFavorite ? "qrc:/icons/bookmark-active.svg" : root.theme.iconSource("bookmark")
                text: root.backend.selectedFavorite ? qsTr("Remove from favorites") : qsTr("Add to favorites")
                Accessible.checkable: true
                Accessible.checked: root.backend.selectedFavorite
                enabled: root.backend.libraryState === Library.Ready && !root.backend.libraryPending && !root.backend.inspecting
                onClicked: root.backend.setSelectedFavorite(!root.backend.selectedFavorite)
            }

            IconButton {
                objectName: "contentRemoveHistory"
                theme: root.theme
                Layout.alignment: Qt.AlignTop
                visible: root.backend.listMode === Search.History && root.selectedInList
                icon.source: root.theme.iconSource("edit-clear")
                text: qsTr("Remove from history")
                enabled: root.backend.libraryState === Library.Ready
                    && !root.backend.libraryPending && !root.backend.inspecting
                onClicked: root.backend.removeHistory(root.backend.selectedWorkId)
            }
        }

        RowLayout {
            spacing: 12

            ActionButton {
                objectName: "contentDownload"
                theme: root.theme
                text: root.selectedForReading ? qsTr("Read")
                    : root.backend.selectedVariantKind === "video" ? qsTr("Watch") : qsTr("Download")
                icon.source: root.theme.iconSource(root.selectedForReading ? "help-contents"
                    : root.backend.selectedVariantKind === "video" ? "torrent-start" : "download")
                highlighted: true
                enabled: root.backend.selectedVariantState === Search.Available && !root.backend.resolving && root.selectedRow >= 0
                onClicked: root.openSelected()
            }

            BusyIndicator {
                Layout.preferredWidth: 24
                Layout.preferredHeight: 24
                visible: root.backend.resolving
                running: visible
            }
        }

        Label {
            objectName: "contentUnavailableVariant"
            Layout.fillWidth: true
            visible: root.backend.selectedVariantState === Search.Unavailable
                || root.backend.selectedVariantState === Search.SourceDisabled
            text: root.backend.selectedVariantState === Search.SourceDisabled
                ? qsTr("Enable this source in Sources.") : qsTr("Saved variant is unavailable. Choose another.")
            wrapMode: Text.Wrap
            textFormat: Text.PlainText
        }

        Label {
            Layout.fillWidth: true
            visible: text.length > 0
            text: root.backend.selectedWorkDescription
            color: root.theme.muted
            wrapMode: Text.Wrap
            maximumLineCount: 3
            elide: Text.ElideRight
            textFormat: Text.PlainText
        }

        ProgressBar {
            Layout.fillWidth: true
            Layout.preferredHeight: 3
            visible: root.backend.inspecting
            indeterminate: true
        }

        VariantFilters {
            Layout.fillWidth: true
            Layout.fillHeight: false
            backend: root.backend
            variants: filteredVariants
            theme: root.theme
            backdrop: root.backdrop
        }

        Label {
            Layout.fillWidth: true
            visible: root.backend.selectedVariantState === Search.Available && root.selectedRow < 0
            text: qsTr("Selected variant is hidden by filters")
            color: root.theme.muted
            wrapMode: Text.Wrap
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: 12
            Layout.rightMargin: 12
            visible: variants.count > 0
            spacing: 16

            Label {
                Layout.fillWidth: true
                text: qsTr("Variant")
                color: root.theme.muted
            }

            Label {
                Layout.preferredWidth: root.sourceWidth
                text: qsTr("Source")
                color: root.theme.muted
            }

            Label {
                Layout.preferredWidth: root.sizeWidth
                visible: root.showTorrentColumns
                text: qsTr("Size")
                color: root.theme.muted
                horizontalAlignment: Text.AlignRight
            }

            Label {
                Layout.preferredWidth: root.seedersWidth
                visible: root.showTorrentColumns
                text: qsTr("Seeds")
                color: root.theme.muted
                horizontalAlignment: Text.AlignRight
            }
        }

        ListView {
            id: variants
            objectName: "contentVariants"
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: filteredVariants
            currentIndex: filteredVariants.count > 0 && root.backend.selectedVariantState === Search.Available
                ? root.selectedRow : -1
            spacing: 2
            activeFocusOnTab: true
            keyNavigationEnabled: true
            reuseItems: true
            ScrollBar.vertical: ScrollBar {}

            function selectCurrentVariant(): void {
                if (activeFocus && currentIndex >= 0)
                    root.backend.selectVariant(filteredVariants.idAt(currentIndex));
            }

            Keys.onReturnPressed: root.openSelected()
            Keys.onEnterPressed: root.openSelected()
            Keys.onPressed: event => {
                if (count > 0 && (event.key === Qt.Key_Home || event.key === Qt.Key_End)) {
                    const index = event.key === Qt.Key_Home ? 0 : count - 1;
                    root.backend.selectVariant(filteredVariants.idAt(index));
                    positionViewAtIndex(index, event.key === Qt.Key_Home ? ListView.Beginning : ListView.End);
                    event.accepted = true;
                }
            }
            Keys.onReleased: event => {
                if (event.key === Qt.Key_Up || event.key === Qt.Key_Down)
                    selectCurrentVariant();
            }

            delegate: VariantDelegate {
                id: variant
                theme: root.theme
                sourceWidth: root.sourceWidth
                sizeWidth: root.sizeWidth
                seedersWidth: root.seedersWidth
                showTorrentColumns: root.showTorrentColumns
                highlighted: root.backend.selectedVariantState === Search.Available && root.backend.selectedVariantId === variant.variantId
                onClicked: {
                    variants.forceActiveFocus();
                    root.backend.selectVariant(variant.variantId);
                }

                onDoubleClicked: root.openSelected()
            }

            Label {
                anchors.centerIn: parent
                visible: !root.backend.inspecting && variants.count === 0 && root.backend.errorText === ""
                text: qsTr("No variants found")
                color: root.theme.muted
            }
        }
    }
}

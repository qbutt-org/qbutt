// SPDX-License-Identifier: GPL-2.0-or-later
// Design Studio fixture only. Production imports the Rust ContentController.
import QtQuick

QtObject {
    id: root

    property bool busy: false
    property bool inspecting: false
    property bool resolving: false
    property string errorText: ""
    property string selectedWorkId: "frieren"
    property string selectedWorkTitle: "Frieren: Beyond Journey's End"
    property string selectedWorkDescription: "An elven mage retraces a journey she once shared with her companions."
    property string searchText: ""

    readonly property var catalogue: [
        {
            workId: "frieren",
            title: "Frieren: Beyond Journey's End",
            source: "AniLiberty",
            category: "anime",
            year: "2023"
        },
        {
            workId: "apothecary",
            title: "The Apothecary Diaries",
            source: "AniLiberty",
            category: "anime",
            year: "2023"
        },
        {
            workId: "dungeon",
            title: "Delicious in Dungeon",
            source: "AniLiberty",
            category: "anime",
            year: "2024"
        }
    ]

    readonly property ListModel works: ListModel {}
    readonly property ListModel variants: ListModel {}

    readonly property Timer searchDelay: Timer {
        interval: 450
        onTriggered: {
            for (const work of root.catalogue) {
                if (work.title.toLowerCase().includes(root.searchText))
                    root.works.append(work);
            }
            root.busy = false;
        }
    }

    readonly property Timer inspectionDelay: Timer {
        interval: 350
        onTriggered: {
            root.populateVariants();
            root.inspecting = false;
        }
    }

    function search(text: string): void {
        cancelSearch();
        inspectionDelay.stop();
        inspecting = false;
        selectedWorkId = "";
        selectedWorkTitle = "";
        selectedWorkDescription = "";
        variants.clear();
        works.clear();
        errorText = "";
        searchText = text.trim().toLowerCase();
        busy = true;
        searchDelay.start();
    }

    function cancelSearch(): void {
        searchDelay.stop();
        busy = false;
    }

    function selectWork(workId: string): void {
        if (selectedWorkId === workId)
            return;

        for (const work of catalogue) {
            if (work.workId !== workId)
                continue;

            selectedWorkId = work.workId;
            selectedWorkTitle = work.title;
            selectedWorkDescription = "";
            variants.clear();
            errorText = "";
            inspecting = true;
            inspectionDelay.restart();
            return;
        }
    }

    function resolveVariant(variantId: string): void {
        if (variantId !== "")
            errorText = "Downloads are disabled in the design preview.";
    }

    function populateVariants(): void {
        for (let episode = 1; episode <= 12; ++episode) {
            variants.append({
                variantId: selectedWorkId + "-" + episode,
                label: "Episode " + episode,
                source: "AniLiberty",
                kind: "torrent",
                quality: "1080p",
                sizeText: "1.2 GB",
                seedersText: String(70 - episode)
            });
        }
    }

    Component.onCompleted: {
        for (const work of catalogue)
            works.append(work);
        populateVariants();
    }
}

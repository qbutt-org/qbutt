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
    padding: 0

    readonly property int sourceWidth: 116
    readonly property int sizeWidth: 80
    readonly property int seedersWidth: 48

    function downloadSelected(): void {
        const selected = variants.currentItem as VariantDelegate;
        if (selected && !backend.resolving)
            backend.resolveVariant(selected.variantId);
    }

    Connections {
        target: root.backend

        function onSelectedWorkIdChanged(): void {
            variants.currentIndex = -1;
        }
    }

    background: Item {}

    contentItem: ColumnLayout {
        visible: root.backend.selectedWorkId !== ""
        spacing: 16

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

        RowLayout {
            spacing: 12

            ActionButton {
                objectName: "contentDownload"
                theme: root.theme
                text: qsTr("Download")
                icon.source: root.theme.iconSource("download")
                highlighted: true
                enabled: variants.currentItem !== null && !root.backend.resolving
                onClicked: root.downloadSelected()
            }

            BusyIndicator {
                Layout.preferredWidth: 24
                Layout.preferredHeight: 24
                visible: root.backend.resolving
                running: visible
            }
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
                text: qsTr("Size")
                color: root.theme.muted
                horizontalAlignment: Text.AlignRight
            }

            Label {
                Layout.preferredWidth: root.seedersWidth
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
            model: root.backend.variants
            currentIndex: -1
            spacing: 2
            activeFocusOnTab: true
            keyNavigationEnabled: true
            reuseItems: true
            ScrollBar.vertical: ScrollBar {}

            onCountChanged: {
                if (count > 0 && currentIndex < 0)
                    currentIndex = 0;
            }

            Keys.onReturnPressed: root.downloadSelected()
            Keys.onEnterPressed: root.downloadSelected()
            Keys.onPressed: event => {
                if (count > 0 && (event.key === Qt.Key_Home || event.key === Qt.Key_End)) {
                    currentIndex = event.key === Qt.Key_Home ? 0 : count - 1;
                    positionViewAtIndex(currentIndex, event.key === Qt.Key_Home ? ListView.Beginning : ListView.End);
                    event.accepted = true;
                }
            }

            delegate: VariantDelegate {
                id: variant
                theme: root.theme
                sourceWidth: root.sourceWidth
                sizeWidth: root.sizeWidth
                seedersWidth: root.seedersWidth
                onClicked: {
                    variants.currentIndex = variant.index;
                    variants.forceActiveFocus();
                }

                onDoubleClicked: root.downloadSelected()
            }

            Label {
                anchors.centerIn: parent
                visible: !root.backend.inspecting && variants.count === 0 && root.backend.errorText === ""
                text: qsTr("No downloads available")
                color: root.theme.muted
            }
        }
    }
}

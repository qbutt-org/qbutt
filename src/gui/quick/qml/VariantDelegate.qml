// SPDX-License-Identifier: GPL-2.0-or-later
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts

ItemDelegate {
    id: root

    required property int index
    required property string variantId
    required property string label
    required property string source
    required property string kind
    required property string quality
    required property string sizeText
    required property string seedersText
    required property Theme theme
    required property int sourceWidth
    required property int sizeWidth
    required property int seedersWidth

    width: ListView.view.width
    padding: 12
    focusPolicy: Qt.NoFocus
    highlighted: ListView.isCurrentItem
    Accessible.name: label

    contentItem: RowLayout {
        spacing: 16

        RowLayout {
            Layout.fillWidth: true
            spacing: 10

            Image {
                Layout.preferredWidth: 18
                Layout.preferredHeight: 18
                visible: root.kind === "torrent"
                source: root.theme.iconSource("torrent-magnet")
                sourceSize: Qt.size(18, 18)
            }

            Label {
                Layout.fillWidth: true
                text: root.quality.length > 0 ? qsTr("%1 [%2]").arg(root.label).arg(root.quality) : root.label
                wrapMode: Text.Wrap
                maximumLineCount: 2
                elide: Text.ElideRight
                textFormat: Text.PlainText
            }
        }

        Label {
            Layout.preferredWidth: root.sourceWidth
            text: root.source
            color: root.theme.muted
            elide: Text.ElideRight
            textFormat: Text.PlainText
        }

        Label {
            Layout.preferredWidth: root.sizeWidth
            text: root.sizeText
            horizontalAlignment: Text.AlignRight
            textFormat: Text.PlainText
        }

        Label {
            Layout.preferredWidth: root.seedersWidth
            text: root.seedersText
            horizontalAlignment: Text.AlignRight
            textFormat: Text.PlainText
        }
    }

    background: Rectangle {
        radius: root.theme.radius
        color: root.highlighted ? root.theme.selection : root.hovered ? root.theme.hover : "transparent"
        border.width: root.ListView.view.activeFocus && root.ListView.isCurrentItem ? 1 : 0
        border.color: root.theme.accent
    }
}

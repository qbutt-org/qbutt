// SPDX-License-Identifier: GPL-2.0-or-later
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts

ItemDelegate {
    id: root

    required property string workId
    required property string title
    required property string source
    required property string category
    required property string year
    required property Theme theme

    readonly property string categoryText: {
        if (category === "anime")
            return qsTr("Anime");
        if (category === "manga")
            return qsTr("Manga");

        return category;
    }

    width: ListView.view.width
    padding: 12
    focusPolicy: Qt.NoFocus
    Accessible.name: title
    Accessible.selected: highlighted

    contentItem: ColumnLayout {
        spacing: 6

        Label {
            Layout.fillWidth: true
            text: root.title
            font.weight: Font.DemiBold
            wrapMode: Text.Wrap
            maximumLineCount: 2
            elide: Text.ElideRight
            textFormat: Text.PlainText
        }

        Label {
            Layout.fillWidth: true
            color: root.theme.muted
            text: root.year.length > 0 ? qsTr("%1, %2").arg(root.categoryText).arg(root.year) : root.categoryText
            elide: Text.ElideRight
            textFormat: Text.PlainText
        }

        Label {
            Layout.fillWidth: true
            color: root.theme.muted
            text: root.source
            elide: Text.ElideRight
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

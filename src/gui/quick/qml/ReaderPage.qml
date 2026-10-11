// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Qbutt.Content

Page {
    id: root
    objectName: "contentQuickReader"

    required property ContentController backend
    required property Theme theme

    padding: 24
    focus: true
    palette.windowText: theme.foreground
    palette.text: theme.foreground

    background: Rectangle {
        color: root.theme.background
    }

    contentItem: ColumnLayout {
        spacing: 20

        ReaderToolbar {
            Layout.fillWidth: true
            backend: root.backend
            theme: root.theme
            onChaptersRequested: chapters.open()
        }

        ReaderCanvas {
            Layout.fillWidth: true
            Layout.fillHeight: true
            backend: root.backend
            theme: root.theme
        }
    }

    ChapterPicker {
        id: chapters
        backend: root.backend
        theme: root.theme
        backdrop: root
    }

    Shortcut {
        sequences: ["Left", "PgUp"]
        enabled: root.visible && !chapters.visible && !root.backend.readerBusy
        onActivated: root.backend.turnReaderPage(-1)
    }

    Shortcut {
        sequences: ["Right", "PgDown"]
        enabled: root.visible && !chapters.visible && !root.backend.readerBusy
        onActivated: root.backend.turnReaderPage(1)
    }

    Shortcut {
        sequence: "Home"
        enabled: root.visible && !chapters.visible && !root.backend.readerBusy
        onActivated: root.backend.selectReaderPage(0)
    }

    Shortcut {
        sequence: "End"
        enabled: root.visible && !chapters.visible && !root.backend.readerBusy
        onActivated: root.backend.selectReaderPage(root.backend.readerPageCount - 1)
    }

    Shortcut {
        sequences: [StandardKey.Find]
        enabled: root.visible && root.backend.readerChapterId !== ""
        onActivated: chapters.open()
    }
}

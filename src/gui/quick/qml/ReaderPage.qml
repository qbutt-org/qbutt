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

    function selectPage(index: int): void {
        canvas.saveScroll();
        backend.selectReaderPage(index);
        canvas.forceActiveFocus();
    }

    contentItem: ColumnLayout {
        spacing: 20

        ReaderToolbar {
            Layout.fillWidth: true
            backend: root.backend
            theme: root.theme
            onBackRequested: {
                canvas.saveScroll();
                root.backend.closeReader();
            }
            onChaptersRequested: chapters.open()
            onChapterTurnRequested: offset => {
                canvas.saveScroll();
                root.backend.turnReaderChapter(offset);
                canvas.forceActiveFocus();
            }
            onPageRequested: index => root.selectPage(index)
            onFitRequested: fit => {
                canvas.saveScroll();
                root.backend.setReaderFit(fit);
                canvas.forceActiveFocus();
            }
            onModeRequested: mode => {
                canvas.saveScroll();
                root.backend.setReaderMode(mode);
                canvas.forceActiveFocus();
            }
        }

        RowLayout {
            Layout.fillWidth: true
            visible: root.backend.readerSettingsState === Reader.Failed
                && root.backend.libraryState === Library.Ready
            spacing: 16

            Label {
                objectName: "readerSettingsError"
                Layout.fillWidth: true
                text: root.backend.readerSettingsErrorText
                textFormat: Text.PlainText
            }

            ActionButton {
                objectName: "readerSettingsRetry"
                theme: root.theme
                text: qsTr("Try again")
                onClicked: {
                    root.backend.retryReaderSettings();
                    canvas.forceActiveFocus();
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            visible: root.backend.readerProgressErrorText !== ""
            spacing: 16

            Label {
                objectName: "readerProgressError"
                Layout.fillWidth: true
                text: root.backend.readerProgressErrorText
                wrapMode: Text.Wrap
                textFormat: Text.PlainText
            }

            ActionButton {
                objectName: "readerProgressRetry"
                theme: root.theme
                text: qsTr("Try again")
                enabled: !root.backend.readerProgressPending
                onClicked: root.backend.retryReaderProgress()
            }
        }

        ReaderCanvas {
            id: canvas
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
        onClosed: canvas.forceActiveFocus()
        onChapterSelected: id => {
            canvas.saveScroll();
            root.backend.selectChapter(id);
        }
    }

    Shortcut {
        sequences: root.backend.readerMode === Reader.Paged ? ["Left", "PgUp"] : ["Left"]
        enabled: root.visible && !root.backend.readerIsText && !chapters.visible && !root.backend.readerBusy
        onActivated: root.selectPage(root.backend.readerPageIndex - 1)
    }

    Shortcut {
        sequences: root.backend.readerMode === Reader.Paged ? ["Right", "PgDown"] : ["Right"]
        enabled: root.visible && !root.backend.readerIsText && !chapters.visible && !root.backend.readerBusy
        onActivated: root.selectPage(root.backend.readerPageIndex + 1)
    }

    Shortcut {
        sequence: "Home"
        enabled: root.visible && !root.backend.readerIsText && !chapters.visible && !root.backend.readerBusy
        onActivated: root.selectPage(0)
    }

    Shortcut {
        sequence: "End"
        enabled: root.visible && !root.backend.readerIsText && !chapters.visible && !root.backend.readerBusy
        onActivated: root.selectPage(root.backend.readerPageCount - 1)
    }

    Shortcut {
        sequences: [StandardKey.Find]
        enabled: root.visible && root.backend.readerHasChapters && !root.backend.readerBusy
        onActivated: chapters.open()
    }
}

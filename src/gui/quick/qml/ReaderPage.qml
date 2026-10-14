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
    readonly property int contentMargin: 48
    readonly property color defaultReadingBackground: theme.dark ? "#202520" : "#f7f5f0"
    readonly property color defaultReadingForeground: theme.dark ? "#e7e9e3" : "#242923"
    readonly property color readingBackground: backend.readerBackgroundColor !== ""
        ? backend.readerBackgroundColor : defaultReadingBackground
    readonly property color chromeForeground: backend.readerBackgroundColor !== ""
        ? contrastingText(readingBackground) : defaultReadingForeground
    readonly property color readingForeground: backend.readerIsText && backend.readerTextColor !== ""
        ? backend.readerTextColor : chromeForeground

    function contrastingText(background: color): color {
        const linear = value => value <= 0.04045 ? value / 12.92
            : Math.pow((value + 0.055) / 1.055, 2.4);
        const luminance = linear(background.r) * 0.2126
            + linear(background.g) * 0.7152 + linear(background.b) * 0.0722;
        return luminance > 0.179 ? "#000000" : "#ffffff";
    }

    focus: true
    palette.windowText: chromeForeground
    palette.text: chromeForeground

    background: Rectangle {
        color: root.readingBackground
    }

    function selectPage(index: int): void {
        canvas.saveScroll();
        backend.selectReaderPage(index);
        canvas.forceActiveFocus();
    }

    function turn(offset: int): void {
        if (backend.readerBusy)
            return;

        if (backend.readerIsText) {
            const next = backend.readerChapterIndex + offset;
            if (next < 0 || next >= backend.readerChapterCount)
                return;
            canvas.saveScroll();
            backend.turnReaderChapter(offset);
        } else {
            const next = backend.readerPageIndex + offset;
            if (next < 0 || next >= backend.readerPageCount)
                return;
            selectPage(next);
        }
        canvas.forceActiveFocus();
    }

    contentItem: Item {
        ColumnLayout {
            id: readerLayout
            anchors.fill: parent
            anchors.leftMargin: root.contentMargin
            anchors.rightMargin: root.contentMargin
            anchors.topMargin: 12
            anchors.bottomMargin: 16
            spacing: 12

            ReaderToolbar {
                id: toolbar
                Layout.fillWidth: true
                backend: root.backend
                theme: root.theme
                foreground: root.chromeForeground
                onBackRequested: {
                    canvas.saveScroll();
                    root.backend.closeReader();
                }
                onChaptersRequested: chapters.open()
                onAppearanceRequested: appearance.open()
                onTranslationSettingsRequested: translationSettings.open()
                onTranslationRequested: translated => {
                    canvas.saveScroll();
                    if (root.backend.readerIsText)
                        root.backend.setReaderTextTranslation(translated);
                    else
                        root.backend.setReaderMangaTranslation(translated);
                    canvas.forceActiveFocus();
                }
            }

            Rectangle {
                id: progressTrack
                Layout.fillWidth: true
                Layout.preferredHeight: 2
                color: Qt.rgba(root.chromeForeground.r, root.chromeForeground.g,
                    root.chromeForeground.b, 0.12)

                Rectangle {
                    width: parent.width * canvas.readingProgress
                    height: parent.height
                    color: root.theme.accent
                }
            }

            RowLayout {
                Layout.fillWidth: true
                visible: root.backend.readerTranslationError
                spacing: 16

                Label {
                    objectName: "readerTranslationError"
                    Layout.fillWidth: true
                    text: root.backend.readerTranslationImageSizeError
                        ? qsTr("This page is too large to translate.")
                        : root.backend.readerTranslationNeedsCodex
                            ? qsTr("Install or sign in to Codex to translate.")
                            : qsTr("Translation failed.")
                    textFormat: Text.PlainText
                }

                ActionButton {
                    objectName: "readerTranslationRetry"
                    visible: !root.backend.readerTranslationImageSizeError
                    theme: root.theme
                    text: qsTr("Try again")
                    onClicked: {
                        if (root.backend.readerIsText)
                            root.backend.setReaderTextTranslation(true);
                        else
                            root.backend.setReaderMangaTranslation(true);
                    }
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
                readingForeground: root.readingForeground
            }
        }

        MouseArea {
            x: 0
            y: readerLayout.y + toolbar.height + progressTrack.height + readerLayout.spacing * 2
            width: root.contentMargin
            height: root.height - y
            enabled: !chapters.visible && !appearance.visible && !translationSettings.visible
                && !root.backend.readerBusy
            onClicked: root.turn(-1)
        }

        MouseArea {
            x: root.width - root.contentMargin
            y: readerLayout.y + toolbar.height + progressTrack.height + readerLayout.spacing * 2
            width: root.contentMargin
            height: root.height - y
            enabled: !chapters.visible && !appearance.visible && !translationSettings.visible
                && !root.backend.readerBusy
            onClicked: root.turn(1)
        }

        Rectangle {
            id: navigation
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.margins: 24
            width: 100
            height: 50
            radius: height / 2
            color: Qt.tint(root.readingBackground, Qt.rgba(root.chromeForeground.r,
                root.chromeForeground.g, root.chromeForeground.b, 0.09))
            visible: !root.backend.readerBusy && (root.backend.readerIsText
                ? root.backend.readerChapterCount > 1 : root.backend.readerPageCount > 1)

            RowLayout {
                anchors.fill: parent
                spacing: 0

                ReaderChromeButton {
                    id: previous
                    objectName: "readerPreviousPage"
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    theme: root.theme
                    foreground: root.chromeForeground
                    icon.source: root.theme.iconSource("chevron-left")
                    horizontalPadding: 0
                    leftCornerRadius: navigation.radius
                    rightCornerRadius: 0
                    Accessible.name: root.backend.readerIsText
                        ? qsTr("Previous chapter") : qsTr("Previous page")
                    enabled: root.backend.readerIsText
                        ? root.backend.readerChapterIndex > 0 : root.backend.readerPageIndex > 0
                    opacity: enabled ? 1 : 0.38
                    onClicked: root.turn(-1)

                    ToolTip {
                        parent: previous
                        visible: previous.hovered
                        text: previous.Accessible.name
                    }
                }

                ReaderChromeButton {
                    id: next
                    objectName: "readerNextPage"
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    theme: root.theme
                    foreground: root.chromeForeground
                    icon.source: root.theme.iconSource("chevron-right")
                    horizontalPadding: 0
                    leftCornerRadius: 0
                    rightCornerRadius: navigation.radius
                    Accessible.name: root.backend.readerIsText
                        ? qsTr("Next chapter") : qsTr("Next page")
                    enabled: root.backend.readerIsText
                        ? root.backend.readerChapterIndex + 1 < root.backend.readerChapterCount
                        : root.backend.readerPageIndex + 1 < root.backend.readerPageCount
                    opacity: enabled ? 1 : 0.38
                    onClicked: root.turn(1)

                    ToolTip {
                        parent: next
                        visible: next.hovered
                        text: next.Accessible.name
                    }
                }
            }
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

    ReaderAppearancePicker {
        id: appearance
        backend: root.backend
        theme: root.theme
        backdrop: root
        effectiveBackground: root.readingBackground
        effectiveTextColor: root.readingForeground
        onAppearanceChanging: {
            if (root.backend.readerIsText)
                canvas.prepareAppearanceChange();
            else
                canvas.saveScroll();
        }
        onFitRequested: fit => {
            canvas.saveScroll();
            root.backend.setReaderFit(fit);
        }
        onModeRequested: mode => {
            canvas.saveScroll();
            root.backend.setReaderMode(mode);
        }
        onClosed: canvas.forceActiveFocus()
    }

    ReaderTranslationPicker {
        id: translationSettings
        backend: root.backend
        theme: root.theme
        backdrop: root
        onClosed: canvas.forceActiveFocus()
    }

    Shortcut {
        sequences: root.backend.readerMode === Reader.Paged ? ["Left", "PgUp"] : ["Left"]
        enabled: root.visible && !root.backend.readerIsText && !chapters.visible
            && !translationSettings.visible && !appearance.visible && !root.backend.readerBusy
        onActivated: root.selectPage(root.backend.readerPageIndex - 1)
    }

    Shortcut {
        sequences: root.backend.readerMode === Reader.Paged ? ["Right", "PgDown"] : ["Right"]
        enabled: root.visible && !root.backend.readerIsText && !chapters.visible
            && !translationSettings.visible && !appearance.visible && !root.backend.readerBusy
        onActivated: root.selectPage(root.backend.readerPageIndex + 1)
    }

    Shortcut {
        sequence: "Home"
        enabled: root.visible && !root.backend.readerIsText && !chapters.visible
            && !translationSettings.visible && !appearance.visible && !root.backend.readerBusy
        onActivated: root.selectPage(0)
    }

    Shortcut {
        sequence: "End"
        enabled: root.visible && !root.backend.readerIsText && !chapters.visible
            && !translationSettings.visible && !appearance.visible && !root.backend.readerBusy
        onActivated: root.selectPage(root.backend.readerPageCount - 1)
    }

    Shortcut {
        sequences: [StandardKey.Find]
        enabled: root.visible && root.backend.readerHasChapters && !translationSettings.visible
            && !appearance.visible && !root.backend.readerBusy
        onActivated: chapters.open()
    }
}

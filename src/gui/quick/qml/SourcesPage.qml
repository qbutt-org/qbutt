// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import Qbutt.Content

Page {
    id: root
    objectName: "contentSourcesPage"

    required property ContentController backend

    padding: 16
    palette.window: applicationPalette.window
    palette.windowText: applicationPalette.windowText

    SystemPalette {
        id: applicationPalette
        colorGroup: SystemPalette.Active
    }

    Theme {
        id: appearance
        background: root.palette.window
        foreground: root.palette.windowText
    }

    contentItem: SourceList {
        backend: root.backend
        theme: appearance
    }
}

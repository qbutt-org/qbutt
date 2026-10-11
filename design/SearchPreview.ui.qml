// SPDX-License-Identifier: GPL-2.0-or-later
import QtQuick
import Qbutt.Content
import "../src/gui/quick/qml" as Product

Item {
    width: 1440
    height: 800

    ContentController {
        id: previewContent
    }

    Product.SearchPage {
        anchors.fill: parent
        backend: previewContent
        iconsBaseUrl: Qt.resolvedUrl("../src/icons/")
        palette.window: "#181b20"
        palette.windowText: "#f2f4f7"
        palette.base: "#20242a"
        palette.text: "#f2f4f7"
        palette.button: "#292e36"
        palette.buttonText: "#f2f4f7"
        palette.highlightedText: "#ffffff"
        palette.placeholderText: "#9ca5b2"
    }
}

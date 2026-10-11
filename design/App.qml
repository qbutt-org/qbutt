// SPDX-License-Identifier: GPL-2.0-or-later
import QtQuick
import QtQuick.Controls.Basic

ApplicationWindow {
    width: 1440
    height: 800
    visible: true
    title: "qbutt — design preview"

    SearchPreview {
        anchors.fill: parent
    }
}

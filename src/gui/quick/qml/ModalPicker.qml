// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Effects

Popup {
    id: root

    required property Theme theme
    required property Item backdrop

    parent: Overlay.overlay
    anchors.centerIn: parent
    width: Math.min(680, parent.width - 96)
    height: Math.min(620, parent.height - 96)
    padding: 24
    modal: true
    dim: true
    focus: true
    popupType: Popup.Item
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
    palette.windowText: theme.foreground
    palette.text: theme.foreground
    palette.buttonText: theme.foreground
    palette.highlight: theme.accent

    background: Rectangle {
        radius: root.theme.radius * 2
        color: root.theme.background
    }

    Overlay.modal: Item {
        MultiEffect {
            anchors.fill: parent
            source: root.backdrop
            blurEnabled: true
            blur: 1
            blurMax: 16
            autoPaddingEnabled: false
        }

        Rectangle {
            anchors.fill: parent
            color: "#73000000"
        }
    }

    enter: Transition {
        OpacityAnimator {
            from: 0
            to: 1
            duration: 140
        }
    }

    exit: Transition {
        OpacityAnimator {
            from: 1
            to: 0
            duration: 100
        }
    }
}

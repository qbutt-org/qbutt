// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic

ComboBox {
    id: control

    required property Theme theme

    leftPadding: 14
    rightPadding: 40
    hoverEnabled: true
    font.pixelSize: theme.controlFontSize

    contentItem: Label {
        text: control.displayText
        color: control.enabled ? control.theme.foreground : control.theme.disabledText
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
        textFormat: Text.PlainText
    }

    indicator: Image {
        x: control.width - width - 12
        y: (control.height - height) / 2
        width: 18
        height: 18
        source: control.theme.iconSource("chevron-down")
        sourceSize: Qt.size(18, 18)
        opacity: control.enabled ? 1 : 0.4
        Accessible.ignored: true
    }

    background: Rectangle {
        implicitWidth: 176
        implicitHeight: control.theme.controlHeight
        radius: control.theme.radius
        color: control.down ? control.theme.controlPressed
            : control.hovered && control.enabled ? control.theme.controlHovered : control.theme.controlBackground
        border.width: control.visualFocus ? 2 : 1
        border.color: control.visualFocus ? control.theme.accent
            : control.hovered && control.enabled ? control.theme.controlBorderHovered : control.theme.controlBorder
    }

    delegate: ItemDelegate {
        id: choice
        required property var model
        required property int index

        width: ListView.view.width
        height: control.theme.controlHeight
        padding: 12
        text: model[control.textRole]
        font.pixelSize: control.theme.controlFontSize
        font.weight: control.currentIndex === index ? Font.DemiBold : Font.Normal
        palette.text: control.theme.foreground
        palette.highlightedText: control.theme.foreground
        highlighted: control.highlightedIndex === index
        hoverEnabled: control.hoverEnabled

        background: Rectangle {
            radius: 4
            color: choice.highlighted ? control.theme.selection : choice.hovered ? control.theme.hover : "transparent"
        }
    }

    popup: Popup {
        y: control.height + 6
        width: control.width
        implicitHeight: contentItem.implicitHeight + topPadding + bottomPadding
        padding: 6

        contentItem: ListView {
            clip: true
            implicitHeight: contentHeight
            model: control.delegateModel
            currentIndex: control.highlightedIndex
        }

        background: Rectangle {
            color: control.theme.background
            radius: control.theme.radius
            border.color: control.theme.controlBorder
        }
    }
}

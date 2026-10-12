// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Qbutt.Content

ModalPicker {
    id: root
    objectName: "contentSourcePicker"

    required property ContentController backend

    width: Math.min(600, parent.width - 96)
    height: Math.min(contentItem.implicitHeight + topPadding + bottomPadding, parent.height - 96)

    contentItem: ColumnLayout {
        spacing: 24

        RowLayout {
            spacing: 16

            Label {
                Layout.fillWidth: true
                text: qsTr("Sources")
                font.pixelSize: 24
                font.weight: Font.DemiBold
            }

            ActionButton {
                objectName: "contentSourcePickerClose"
                theme: root.theme
                text: qsTr("Close")
                onClicked: root.close()
            }
        }

        SourceList {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumHeight: 0
            backend: root.backend
            theme: root.theme
        }
    }
}

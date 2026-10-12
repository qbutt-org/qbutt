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

        BusyIndicator {
            objectName: "contentSourcesLoading"
            Layout.alignment: Qt.AlignHCenter
            visible: root.backend.sourceSettingsState === Sources.Loading
            running: visible
        }

        Repeater {
            model: root.backend.sources

            delegate: ColumnLayout {
                id: sourceRow
                required property var model

                Layout.fillWidth: true
                spacing: 6

                RowLayout {
                    spacing: 12

                    Switch {
                        id: sourceToggle
                        objectName: "contentSourceEnabled_" + sourceRow.model.sourceId
                        Layout.fillWidth: true
                        Layout.minimumHeight: root.theme.controlHeight
                        padding: 0
                        spacing: 16
                        text: sourceRow.model.name
                        checked: sourceRow.model.enabled
                        enabled: !sourceRow.model.pending && !sourceRow.model.retrying
                        font.pixelSize: 16
                        palette.windowText: enabled ? root.theme.foreground : root.theme.disabledText
                        palette.dark: root.theme.accent
                        palette.midlight: root.theme.controlPressed
                        palette.mid: root.theme.controlBorder
                        palette.window: root.theme.accentText
                        onClicked: root.backend.setSourceEnabled(sourceRow.model.sourceId, checked)
                    }

                    Image {
                        objectName: "contentSourceFailure_" + sourceRow.model.sourceId
                        Layout.preferredWidth: 20
                        Layout.preferredHeight: 20
                        visible: sourceRow.model.failed
                        source: root.theme.iconSource("error")
                        sourceSize: Qt.size(20, 20)
                        Accessible.ignored: true
                    }

                    BusyIndicator {
                        objectName: "contentSourcePending_" + sourceRow.model.sourceId
                        Layout.preferredWidth: 24
                        Layout.preferredHeight: 24
                        visible: sourceRow.model.pending || sourceRow.model.retrying
                        running: visible
                    }

                    ActionButton {
                        objectName: "contentSourceRetry_" + sourceRow.model.sourceId
                        theme: root.theme
                        visible: sourceRow.model.failed
                        text: qsTr("Try again")
                        Accessible.name: qsTr("Retry %1").arg(sourceRow.model.name)
                        enabled: root.backend.sourceSettingsState !== Sources.Failed
                            && sourceRow.model.enabled && !sourceRow.model.pending && !sourceRow.model.retrying
                        onClicked: root.backend.retrySource(sourceRow.model.sourceId)
                    }
                }

                Label {
                    objectName: "contentSourceFailureText_" + sourceRow.model.sourceId
                    Layout.fillWidth: true
                    Layout.leftMargin: sourceToggle.indicator.width + sourceToggle.spacing
                    visible: sourceRow.model.failed
                    text: sourceRow.model.failureText
                    color: root.theme.muted
                    wrapMode: Text.Wrap
                    textFormat: Text.PlainText
                }
            }
        }

        RowLayout {
            visible: root.backend.sourceSettingsState === Sources.Failed
            spacing: 16

            Label {
                objectName: "contentSourceSettingsError"
                Layout.fillWidth: true
                text: root.backend.sourceSettingsErrorText
                wrapMode: Text.Wrap
                textFormat: Text.PlainText
            }

            ActionButton {
                objectName: "contentSourceSettingsRetry"
                theme: root.theme
                text: qsTr("Try again")
                onClicked: root.backend.retrySourceSettings()
            }
        }
    }
}

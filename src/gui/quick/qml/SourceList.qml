// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Qbutt.Content

ColumnLayout {
    id: root

    required property ContentController backend
    required property Theme theme
    property string expandedSourceId: ""
    property string openFailedSourceId: ""

    spacing: 16

    Connections {
        target: root.backend
        function onSourceAccountOpened(sourceId, opened) {
            root.openFailedSourceId = opened ? "" : sourceId
        }
    }

    BusyIndicator {
        objectName: "contentSourcesLoading"
        Layout.alignment: Qt.AlignHCenter
        visible: root.backend.sourceSettingsState === Sources.Loading
        running: visible
    }

    ScrollView {
        id: sourceScroll
        objectName: "contentSourcesScroll"
        Layout.fillWidth: true
        Layout.fillHeight: true
        Layout.minimumHeight: 0
        implicitHeight: sourceList.implicitHeight
        contentWidth: availableWidth
        clip: true
        ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

        ColumnLayout {
            id: sourceList
            width: sourceScroll.availableWidth
            spacing: 6

            Repeater {
                model: root.backend.sources

                delegate: ColumnLayout {
                    id: sourceRow
                    required property var model

                    Layout.fillWidth: true
                    spacing: 4

                    RowLayout {
                        spacing: 12

                        CheckBox {
                            id: sourceCheck
                            objectName: "contentSourceEnabled_" + sourceRow.model.sourceId
                            Layout.fillWidth: true
                            Layout.minimumHeight: 28
                            padding: 0
                            spacing: 10
                            indicator.width: 20
                            indicator.height: 20
                            text: sourceRow.model.name
                            checked: sourceRow.model.enabled
                            enabled: !sourceRow.model.pending && !sourceRow.model.retrying
                            font.pixelSize: 14
                            palette.windowText: enabled ? root.theme.foreground : root.theme.disabledText
                            palette.text: root.theme.accentText
                            palette.base: checked ? root.theme.accent : root.theme.controlBackground
                            palette.light: checked ? root.theme.accent : root.theme.controlPressed
                            palette.mid: checked ? root.theme.accent : root.theme.controlBorder
                            palette.highlight: root.theme.accent
                            onClicked: root.backend.setSourceEnabled(sourceRow.model.sourceId, checked)
                        }

                        Button {
                            objectName: "contentSourceAccount_" + sourceRow.model.sourceId
                            visible: sourceRow.model.accountSource
                            Layout.preferredHeight: 28
                            padding: 0
                            flat: true
                            text: qsTranslate("SourcePicker", "Account")
                            font.pixelSize: 13
                            palette.buttonText: root.theme.foreground
                            onClicked: {
                                root.expandedSourceId = root.expandedSourceId === sourceRow.model.sourceId
                                    ? "" : sourceRow.model.sourceId
                                root.openFailedSourceId = ""
                            }
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
                            text: sourceRow.model.manualCheck
                                ? qsTranslate("SourcePicker", "Open %1").arg(sourceRow.model.name) : qsTranslate("SourcePicker", "Try again")
                            Accessible.name: sourceRow.model.manualCheck
                                ? qsTranslate("SourcePicker", "Continue in %1").arg(sourceRow.model.name)
                                : qsTranslate("SourcePicker", "Retry %1").arg(sourceRow.model.name)
                            enabled: root.backend.sourceSettingsState !== Sources.Failed
                                && sourceRow.model.enabled && !sourceRow.model.pending && !sourceRow.model.retrying
                            onClicked: root.backend.retrySource(sourceRow.model.sourceId)
                        }
                    }

                    Label {
                        objectName: "contentSourceFailureText_" + sourceRow.model.sourceId
                        Layout.fillWidth: true
                        Layout.leftMargin: sourceCheck.indicator.width + sourceCheck.spacing
                        visible: sourceRow.model.failed
                        text: sourceRow.model.failureText
                        color: root.theme.muted
                        wrapMode: Text.Wrap
                        textFormat: Text.PlainText
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: sourceCheck.indicator.width + sourceCheck.spacing
                        visible: root.expandedSourceId === sourceRow.model.sourceId
                        spacing: 12

                        Label {
                            Layout.fillWidth: true
                            text: root.openFailedSourceId === sourceRow.model.sourceId
                                ? qsTranslate("SourcePicker", "Could not open the browser. Choose another in Options > Search > Browser.")
                                : qsTranslate("SourcePicker", "Uses your browser session when available.")
                            color: root.theme.muted
                            wrapMode: Text.Wrap
                            textFormat: Text.PlainText
                        }

                        ActionButton {
                            objectName: "contentSourceOpen_" + sourceRow.model.sourceId
                            theme: root.theme
                            text: qsTranslate("SourcePicker", "Open %1").arg(sourceRow.model.name)
                            onClicked: {
                                root.openFailedSourceId = ""
                                if (!root.backend.openSourceAccount(sourceRow.model.sourceId))
                                    root.openFailedSourceId = sourceRow.model.sourceId
                            }
                        }
                    }
                }
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
            text: qsTranslate("SourcePicker", "Try again")
            onClicked: root.backend.retrySourceSettings()
        }
    }
}

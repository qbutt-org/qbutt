// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Qbutt.Content

ModalPicker {
    id: root
    objectName: "readerTranslationPicker"

    required property ContentController backend
    readonly property var modelChoices: {
        const choices = [{ title: qsTr("Automatic"), id: "" }];
        const models = backend.readerTranslationModels;
        const selected = backend.readerTranslationModel;
        if (selected !== "" && models.indexOf(selected) < 0)
            choices.push({ title: qsTr("%1 (unavailable)").arg(selected), id: selected });
        for (let index = 0; index < models.length; ++index)
            choices.push({ title: models[index], id: models[index] });
        return choices;
    }
    readonly property var effortChoices: {
        const choices = [];
        const efforts = backend.readerTranslationEfforts;
        const selected = backend.readerTranslationEffort;
        if (selected !== "" && efforts.indexOf(selected) < 0)
            choices.push({ title: qsTr("%1 (unavailable)").arg(selected), id: selected });
        for (let index = 0; index < efforts.length; ++index)
            choices.push({ title: efforts[index], id: efforts[index] });
        return choices;
    }

    anchors.centerIn: undefined
    width: Math.min(420, parent.width - 48)
    height: 280
    x: parent.width - width - 24
    y: (parent.height - height) / 2

    onOpened: backend.loadReaderTranslationOptions()

    contentItem: ColumnLayout {
        spacing: 16

        RowLayout {
            Layout.fillWidth: true

            Label {
                Layout.fillWidth: true
                text: qsTr("Translation settings")
                font.pixelSize: 22
                font.weight: Font.DemiBold
            }

            ReaderChromeButton {
                objectName: "readerTranslationClose"
                theme: root.theme
                foreground: root.theme.foreground
                icon.source: root.theme.iconSource("close")
                horizontalPadding: 0
                Layout.preferredWidth: 42
                Accessible.name: qsTr("Close")
                onClicked: root.close()
            }
        }

        RowLayout {
            Layout.fillWidth: true

            Label {
                Layout.fillWidth: true
                text: qsTr("Translate into")
            }

            SelectField {
                objectName: "readerTranslationLanguage"
                Layout.preferredWidth: 190
                theme: root.theme
                enabled: root.backend.readerSettingsState === Reader.Ready
                Accessible.name: qsTr("Translation language")
                textRole: "title"
                valueRole: "id"
                model: [
                    { title: qsTr("Russian"), id: "ru" },
                    { title: qsTr("English"), id: "en" }
                ]
                currentIndex: root.backend.readerTranslationLanguage === "en" ? 1 : 0
                onActivated: {
                    if (root.backend.setReaderTranslationLanguage(currentValue))
                        root.backend.loadReaderTranslationOptions();
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true

            Label {
                Layout.fillWidth: true
                text: qsTr("Codex model")
            }

            SelectField {
                objectName: "readerTranslationModel"
                Layout.preferredWidth: 190
                theme: root.theme
                enabled: !root.backend.readerTranslationOptionsLoading
                    && !root.backend.readerTranslationOptionsError
                    && root.backend.readerTranslationModels.length > 0
                Accessible.name: qsTr("Codex model")
                textRole: "title"
                valueRole: "id"
                model: root.modelChoices
                currentIndex: root.modelChoices.findIndex(choice =>
                    choice.id === root.backend.readerTranslationModel)
                onActivated: root.backend.setReaderTranslationModel(currentValue)
            }
        }

        RowLayout {
            Layout.fillWidth: true
            visible: root.backend.readerTranslationModel !== ""

            Label {
                Layout.fillWidth: true
                text: qsTr("Reasoning")
            }

            SelectField {
                objectName: "readerTranslationEffort"
                Layout.preferredWidth: 190
                theme: root.theme
                enabled: !root.backend.readerTranslationOptionsLoading
                    && !root.backend.readerTranslationOptionsError
                    && root.backend.readerTranslationEfforts.length > 0
                Accessible.name: qsTr("Reasoning effort")
                textRole: "title"
                valueRole: "id"
                model: root.effortChoices
                currentIndex: root.effortChoices.findIndex(choice =>
                    choice.id === root.backend.readerTranslationEffort)
                onActivated: root.backend.setReaderTranslationEffort(currentValue)
            }
        }

        Label {
            Layout.fillWidth: true
            visible: root.backend.readerTranslationOptionsError
            text: qsTr("Codex models are unavailable. Your saved choice still applies.")
            color: root.theme.muted
            wrapMode: Text.Wrap
        }

        Item { Layout.fillHeight: true }
    }
}

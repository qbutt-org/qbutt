// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import Qbutt.Content

ColumnLayout {
    id: root
    objectName: "contentVariantFilters"

    required property ContentController backend
    required property VariantFilterModel variants
    required property Theme theme
    required property Item backdrop
    enabled: backend.libraryState === Library.Ready

    readonly property bool active: backend.variantEpisodeFilter !== ""
        || backend.variantTranslationFilter !== "" || backend.variantSourceFilter !== ""
        || backend.variantPlayerFilter !== "" || backend.variantLanguageFilter !== ""
        || backend.variantTransportFilter !== "" || backend.variantQuery !== ""
    spacing: 8

    RowLayout {
        Layout.fillWidth: true
        spacing: 8

        FacetField {
            objectName: "contentEpisodeFilter"
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            visible: root.variants.episodeOptions.length > 1 || value.length > 0
            theme: root.theme
            backdrop: root.backdrop
            title: qsTr("Episode")
            allText: qsTr("All episodes")
            value: root.backend.variantEpisodeFilter
            text: value.length > 0 ? qsTr("Episode %1").arg(value) : allText
            options: root.variants.episodeOptions
            onChosen: option => root.backend.setVariantEpisodeFilter(option.value)
        }

        FacetField {
            objectName: "contentTranslationFilter"
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            visible: root.variants.translationOptions.length > 1 || value.length > 0
            theme: root.theme
            backdrop: root.backdrop
            title: qsTr("Translation")
            allText: qsTr("All translations")
            value: root.backend.variantTranslationFilter
            options: root.variants.translationOptions
            onChosen: option => root.backend.setVariantTranslationFilter(option.value)
        }

        FacetField {
            objectName: "contentProviderFilter"
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            visible: root.variants.providerOptions.length > 1 || value.length > 0
            theme: root.theme
            backdrop: root.backdrop
            title: qsTr("Source")
            allText: qsTr("All sources")
            value: {
                const source = String(root.backend.variantSourceFilter);
                const player = String(root.backend.variantPlayerFilter);
                return source.length > 0 || player.length > 0 ? source.length + ":" + source + player : "";
            }
            fallbackText: {
                if (root.backend.variantSourceFilter === "")
                    return root.backend.variantPlayerFilter;

                return root.backend.variantPlayerFilter !== ""
                    ? qsTr("%1 [%2]").arg(root.backend.variantSourceFilter).arg(root.backend.variantPlayerFilter)
                    : root.backend.variantSourceFilter;
            }
            options: root.variants.providerOptions
            onChosen: option => root.backend.setVariantProviderFilter(option.source ?? "", option.player ?? "")
        }

        FacetField {
            objectName: "contentLanguageFilter"
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            visible: root.variants.languageOptions.length > 1 || value.length > 0
            theme: root.theme
            backdrop: root.backdrop
            title: qsTr("Language")
            allText: qsTr("All languages")
            value: root.backend.variantLanguageFilter
            options: root.variants.languageOptions
            onChosen: option => root.backend.setVariantLanguageFilter(option.value)
        }
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: 8

        QueryField {
            objectName: "contentVariantQuery"
            Layout.fillWidth: true
            theme: root.theme
            placeholderText: qsTr("Find a variant")
            Accessible.name: placeholderText
            text: root.backend.variantQuery
            onTextEdited: root.backend.setVariantQuery(text)
        }

        ActionButton {
            objectName: "contentClearVariantFilters"
            theme: root.theme
            visible: root.active
            text: qsTr("Reset filters")
            onClicked: root.backend.clearVariantFilters()
        }
    }
}

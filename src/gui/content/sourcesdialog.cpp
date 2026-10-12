// SPDX-License-Identifier: GPL-2.0-or-later
#include "sourcesdialog.h"

#include <utility>

#include <QAction>
#include <QCheckBox>
#include <QComboBox>
#include <QDialogButtonBox>
#include <QFormLayout>
#include <QHBoxLayout>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QLocale>
#include <QPushButton>
#include <QScrollArea>
#include <QVBoxLayout>

#include "base/content/client.h"
#include "base/global.h"
#include "base/preferences.h"
#include "gui/uithememanager.h"

#include "browserconnectionwidget.h"

Content::SourcesDialog::SourcesDialog(QWidget *parent, const QString &sourceId)
    : QDialog(parent)
    , m_translationOnly {sourceId == u"ai"}
    , m_initialSource {sourceId}
{
    setWindowTitle(m_translationOnly ? tr("AI translation") : tr("Sources"));
    setMinimumSize(560, 360);
    resize(740, 480);

    auto *layout = new QVBoxLayout(this);
    auto *body = new QHBoxLayout;
    auto *sourceList = new QVBoxLayout;
    m_filter = new QLineEdit(this);
    m_filter->setPlaceholderText(tr("Find a source"));
    m_filter->setClearButtonEnabled(true);
    m_filter->setVisible(!m_translationOnly);
    sourceList->addWidget(m_filter);
    m_sources = new QListWidget(this);
    m_sources->setObjectName(u"contentSourceList"_s);
    m_sources->setMinimumWidth(180);
    m_sources->setMaximumWidth(280);
    m_sources->setVisible(!m_translationOnly);
    sourceList->addWidget(m_sources, 1);
    body->addLayout(sourceList, m_translationOnly ? 0 : 1);

    auto *scroll = new QScrollArea(this);
    scroll->setWidgetResizable(true);
    scroll->setFrameShape(QFrame::NoFrame);
    auto *page = new QWidget(scroll);
    auto *pageLayout = new QVBoxLayout(page);
    m_name = new QLabel(page);
    QFont heading = m_name->font();
    heading.setBold(true);
    heading.setPointSize(heading.pointSize() + 2);
    m_name->setFont(heading);
    m_name->setWordWrap(true);
    pageLayout->addWidget(m_name);
    m_health = new QLabel(page);
    m_health->setObjectName(u"contentSourceHealth"_s);
    m_health->setTextFormat(Qt::PlainText);
    m_health->setWordWrap(true);
    m_health->hide();
    pageLayout->addWidget(m_health);
    m_check = new QPushButton(tr("Check again"), page);
    m_check->setObjectName(u"contentSourceCheck"_s);
    m_check->hide();
    pageLayout->addWidget(m_check, 0, Qt::AlignLeft);
    m_enabled = new QCheckBox(tr("Use in search"), page);
    m_enabled->setVisible(!m_translationOnly);
    pageLayout->addWidget(m_enabled);
    m_form = new QFormLayout;
    m_form->setFieldGrowthPolicy(QFormLayout::AllNonFixedFieldsGrow);
    m_form->setRowWrapPolicy(QFormLayout::WrapLongRows);
    pageLayout->addLayout(m_form);
    m_browserConnection = new BrowserConnectionWidget(true, page);
    m_browserConnection->hide();
    pageLayout->addWidget(m_browserConnection);
    m_save = new QPushButton(tr("Save"), page);
    m_save->setEnabled(false);
    pageLayout->addWidget(m_save, 0, Qt::AlignLeft);
    pageLayout->addStretch();
    scroll->setWidget(page);
    body->addWidget(scroll, 2);
    layout->addLayout(body, 1);

    m_status = new QLabel(tr("Loading sources…"), this);
    m_status->setObjectName(u"contentSourcesStatus"_s);
    m_status->setWordWrap(true);
    layout->addWidget(m_status);
    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Close, this);
    m_refresh = buttons->addButton(tr("Update sources"), QDialogButtonBox::ActionRole);
    m_refresh->setVisible(!m_translationOnly);
    layout->addWidget(buttons);
    connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::reject);
    connect(m_sources, &QListWidget::currentRowChanged, this, &SourcesDialog::selectSource);
    connect(m_filter, &QLineEdit::textChanged, this, &SourcesDialog::filterSources);
    connect(m_save, &QPushButton::clicked, this, &SourcesDialog::saveSource);
    connect(m_check, &QPushButton::clicked, this, [this]
    {
        const int row = m_sources->currentRow();
        if ((row < 0) || m_request)
            return;

        m_check->setEnabled(false);
        m_check->setText(tr("Checking…"));
        m_sources->setEnabled(false);
        m_filter->setEnabled(false);
        m_save->setEnabled(false);
        m_refresh->setEnabled(false);
        m_enabled->setEnabled(false);
        for (QWidget *field : m_fields)
            field->setEnabled(false);
        const QJsonObject source = m_catalog.at(row).toObject();
        const bool interactive = source.value(u"health"_s).toObject().value(u"code"_s).toString()
            == u"browser_check_required";
        m_request = Client::instance()->request(u"checkSource"_s,
            {{u"id"_s, source.value(u"id"_s)}, {u"interactive"_s, interactive},
                {u"language"_s, QLocale(Preferences::instance()->getLocale()).bcp47Name()}});
    });
    connect(m_refresh, &QPushButton::clicked, this, [this]
    {
        if (m_request)
            return;

        m_status->setText(tr("Updating sources…"));
        m_refresh->setEnabled(false);
        m_request = Client::instance()->request(u"refresh"_s);
    });
    connect(Client::instance(), &Client::eventReceived, this, &SourcesDialog::handleEvent);
    m_request = Client::instance()->request(u"sources"_s);
}

Content::SourcesDialog::~SourcesDialog()
{
    Client::instance()->cancel(m_request);
}

void Content::SourcesDialog::filterSources()
{
    QListWidgetItem *firstMatch = nullptr;
    for (int row = 0; row < m_sources->count(); ++row)
    {
        QListWidgetItem *item = m_sources->item(row);
        const bool matches = item->text().contains(m_filter->text(), Qt::CaseInsensitive);
        item->setHidden(!matches);
        if (matches && !firstMatch)
            firstMatch = item;
    }
    if (!m_sources->currentItem() || m_sources->currentItem()->isHidden())
        m_sources->setCurrentItem(firstMatch);
}

void Content::SourcesDialog::selectSource()
{
    while (m_form->rowCount() > 0)
        m_form->removeRow(0);
    m_fields.clear();

    const int row = m_sources->currentRow();
    const QJsonObject source = (row < 0) ? QJsonObject() : m_catalog.at(row).toObject();
    const QJsonObject health = source.value(u"health"_s).toObject();
    const bool failed = health.value(u"status"_s).toString() == u"error";
    const bool browserCheck = health.value(u"code"_s).toString() == u"browser_check_required";
    m_health->setText(health.value(u"message"_s).toString());
    m_health->setVisible(failed);
    m_check->setVisible(failed);
    m_check->setEnabled(!m_request);
    m_check->setText(browserCheck ? tr("Complete check") : tr("Check again"));
    m_check->setToolTip(health.value(u"url"_s).toString());
    m_save->setEnabled((row >= 0) && !m_request);
    m_enabled->setEnabled((row >= 0) && !m_request);
    const bool browser = (row >= 0) && (m_catalog.at(row).toObject().value(u"id"_s).toString() == u"browser");
    m_browserConnection->setVisible(browser);
    m_save->setVisible(!browser);
    if (row < 0)
    {
        m_name->clear();
        m_enabled->hide();
        return;
    }

    const bool translation = source.value(u"id"_s).toString() == u"ai";
    m_name->setText(source.value(u"name"_s).toString());
    m_enabled->setVisible(!source.value(u"categories"_s).toArray().isEmpty());
    m_enabled->setChecked(source.value(u"enabled"_s).toBool());
    if (browser)
        return;

    const QJsonArray settings = source.value(u"settings"_s).toArray();
    QJsonArray orderedSettings;
    for (const bool advanced : {false, true})
    {
        for (const QJsonValue &value : settings)
        {
            if (value.toObject().value(u"advanced"_s).toBool() == advanced)
                orderedSettings.append(value);
        }
    }
    bool advancedHeading = false;
    for (const QJsonValue &value : orderedSettings)
    {
        const QJsonObject setting = value.toObject();
        if (setting.value(u"advanced"_s).toBool() && !advancedHeading)
        {
            auto *heading = new QLabel(tr("Advanced settings"), this);
            heading->setContentsMargins(0, 12, 0, 0);
            QFont font = heading->font();
            font.setBold(true);
            heading->setFont(font);
            m_form->addRow(heading);
            advancedHeading = true;
        }
        const QString name = setting.value(u"name"_s).toString();
        const QString type = setting.value(u"type"_s).toString();
        QWidget *field = nullptr;
        if (type == u"boolean")
        {
            auto *check = new QCheckBox(this);
            check->setChecked(setting.value(u"value"_s).toBool());
            field = check;
        }
        else if (type == u"select")
        {
            auto *combo = new QComboBox(this);
            combo->setSizeAdjustPolicy(QComboBox::AdjustToMinimumContentsLengthWithIcon);
            for (const QJsonValue &optionValue : setting.value(u"options"_s).toArray())
            {
                const QJsonObject option = optionValue.toObject();
                const QString optionId = option.value(u"value"_s).toString();
                QString optionLabel = option.value(u"label"_s).toString();
                if (translation && (optionId == u"auto"))
                    optionLabel = tr("Automatic");
                else if (translation && (name == u"provider") && (optionId == u"openai-compatible"))
                    optionLabel = tr("OpenAI compatible");
                combo->addItem(optionLabel, optionId);
            }
            combo->setCurrentIndex(combo->findData(setting.value(u"value"_s).toString()));
            field = combo;
        }
        else
        {
            auto *edit = new QLineEdit(setting.value(u"value"_s).toString(), this);
            if (type == u"password")
            {
                edit->setEchoMode(QLineEdit::Password);
                if (setting.value(u"configured"_s).toBool())
                {
                    edit->setPlaceholderText(tr("Saved"));
                    QAction *clear = edit->addAction(UIThemeManager::instance()->getIcon(u"edit-clear"_s), QLineEdit::TrailingPosition);
                    clear->setToolTip(tr("Clear saved value"));
                    connect(clear, &QAction::triggered, edit, [edit]
                    {
                        edit->clear();
                        edit->setModified(true);
                        edit->setPlaceholderText(tr("Will be cleared"));
                    });
                }
            }
            field = edit;
        }
        field->setEnabled(!m_request);
        m_fields.insert(name, field);
        QString labelText = setting.value(u"label"_s).toString();
        if (source.value(u"id"_s).toString() == u"hdrezka")
        {
            if (name == u"cookie")
                labelText = tr("Premium account session cookies");
            else if (name == u"baseUrl")
                labelText = tr("Site address");
        }
        else if (translation)
        {
            if (name == u"provider")
                labelText = tr("Provider");
            else if (name == u"apiKey")
                labelText = tr("API key");
            else if (name == u"model")
                labelText = tr("Model");
            else if (name == u"baseUrl")
                labelText = tr("API address");
        }
        auto *label = new QLabel(labelText, this);
        label->setWordWrap(true);
        m_form->addRow(label, field);
    }
}

void Content::SourcesDialog::saveSource()
{
    const int row = m_sources->currentRow();
    if ((row < 0) || m_request)
        return;

    QJsonObject settings;
    for (auto it = m_fields.cbegin(); it != m_fields.cend(); ++it)
    {
        if (const auto *edit = qobject_cast<QLineEdit *>(it.value()))
        {
            if ((edit->echoMode() != QLineEdit::Password) || edit->isModified())
                settings.insert(it.key(), edit->text());
        }
        else if (const auto *check = qobject_cast<QCheckBox *>(it.value()))
            settings.insert(it.key(), check->isChecked());
        else if (const auto *combo = qobject_cast<QComboBox *>(it.value()))
            settings.insert(it.key(), combo->currentData().toString());
    }
    m_save->setEnabled(false);
    m_status->setText(tr("Saving…"));
    m_request = Client::instance()->request(u"configureSource"_s, {
        {u"id"_s, m_catalog.at(row).toObject().value(u"id"_s)},
        {u"enabled"_s, m_enabled->isChecked()}, {u"settings"_s, settings}
    });
}

void Content::SourcesDialog::handleEvent(const qint64 id, const QString &event, const QJsonValue &data)
{
    if (id != m_request)
        return;

    if (event == u"details")
    {
        emit publicationInspected(data.toObject());
    }
    else if (event == u"sources")
    {
        const QString selected = m_name->text();
        const QString initialSource = std::exchange(m_initialSource, {});
        m_catalog = {};
        for (const QString &sourceId : {u"browser"_s, u"ai"_s})
        {
            if (m_translationOnly && (sourceId != u"ai"))
                continue;

            for (const QJsonValue &value : data.toArray())
            {
                if (value.toObject().value(u"id"_s).toString() == sourceId)
                {
                    QJsonObject source = value.toObject();
                    source.insert(u"name"_s, (sourceId == u"browser") ? tr("Browser") : tr("AI translation"));
                    m_catalog.append(source);
                }
            }
        }
        for (const QJsonValue &value : data.toArray())
        {
            const QString sourceId = value.toObject().value(u"id"_s).toString();
            if (!m_translationOnly && (sourceId != u"browser") && (sourceId != u"ai"))
                m_catalog.append(value);
        }
        m_sources->clear();
        QListWidgetItem *initialItem = nullptr;
        for (const QJsonValue &value : m_catalog)
        {
            const QJsonObject source = value.toObject();
            auto *item = new QListWidgetItem(source.value(u"name"_s).toString(), m_sources);
            if (source.value(u"id"_s).toString() == initialSource)
                initialItem = item;
            const QJsonObject health = source.value(u"health"_s).toObject();
            if (health.value(u"status"_s).toString() == u"error")
            {
                item->setIcon(UIThemeManager::instance()->getIcon(u"task-reject"_s));
                item->setToolTip(health.value(u"message"_s).toString());
            }
            else
            {
                item->setToolTip(source.value(u"url"_s).toString());
            }
        }
        const QList<QListWidgetItem *> matches = m_sources->findItems(selected, Qt::MatchExactly);
        m_sources->setCurrentItem(initialItem ? initialItem : (matches.isEmpty() ? m_sources->item(0) : matches.first()));
        filterSources();
        m_status->setText(m_translationOnly ? QString() : tr("%n source(s)", nullptr, m_catalog.size()));
    }
    else if (event == u"error")
    {
        m_status->setText(data.toObject().value(u"message"_s).toString());
    }
    if ((event == u"complete") || (event == u"error"))
    {
        m_request = 0;
        const int row = m_sources->currentRow();
        const QJsonObject health = (row < 0) ? QJsonObject()
            : m_catalog.at(row).toObject().value(u"health"_s).toObject();
        m_sources->setEnabled(true);
        m_filter->setEnabled(true);
        m_save->setEnabled(row >= 0);
        m_refresh->setEnabled(true);
        m_enabled->setEnabled(row >= 0);
        for (QWidget *field : m_fields)
            field->setEnabled(true);
        m_check->setEnabled(true);
        m_check->setText((health.value(u"code"_s).toString() == u"browser_check_required")
            ? tr("Complete check") : tr("Check again"));
    }
}

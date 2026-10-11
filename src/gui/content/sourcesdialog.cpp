// SPDX-License-Identifier: GPL-2.0-or-later
#include "sourcesdialog.h"

#include <QAction>
#include <QCheckBox>
#include <QComboBox>
#include <QDialogButtonBox>
#include <QEvent>
#include <QFormLayout>
#include <QHBoxLayout>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QPushButton>
#include <QScrollArea>
#include <QVBoxLayout>

#include "base/content/client.h"
#include "base/global.h"
#include "gui/uithememanager.h"

Content::SourcesDialog::SourcesDialog(QWidget *parent)
    : QDialog(parent)
{
    setWindowTitle(tr("Sources"));
    setMinimumSize(560, 360);
    resize(740, 480);

    auto *layout = new QVBoxLayout(this);
    auto *body = new QHBoxLayout;
    auto *sourceList = new QVBoxLayout;
    m_filter = new QLineEdit(this);
    m_filter->setPlaceholderText(tr("Find a source"));
    m_filter->setClearButtonEnabled(true);
    sourceList->addWidget(m_filter);
    m_sources = new QListWidget(this);
    m_sources->setMinimumWidth(180);
    m_sources->setMaximumWidth(280);
    sourceList->addWidget(m_sources, 1);
    body->addLayout(sourceList, 1);

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
    m_enabled = new QCheckBox(tr("Use in search"), page);
    pageLayout->addWidget(m_enabled);
    m_form = new QFormLayout;
    m_form->setFieldGrowthPolicy(QFormLayout::AllNonFixedFieldsGrow);
    m_form->setRowWrapPolicy(QFormLayout::WrapLongRows);
    pageLayout->addLayout(m_form);
    m_connect = new QPushButton(tr("Install browser extension…"), page);
    m_connect->setObjectName(u"contentBrowserInstall"_s);
    m_connect->hide();
    pageLayout->addWidget(m_connect, 0, Qt::AlignLeft);
    m_browserSetup = new QLabel(page);
    m_browserSetup->setObjectName(u"contentBrowserSetup"_s);
    m_browserSetup->setWordWrap(true);
    m_browserSetup->hide();
    pageLayout->addWidget(m_browserSetup);
    m_extensionDirectory = new QLineEdit(page);
    m_extensionDirectory->setObjectName(u"contentBrowserExtensionDirectory"_s);
    m_extensionDirectory->setReadOnly(true);
    m_extensionDirectory->hide();
    pageLayout->addWidget(m_extensionDirectory);
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
    layout->addWidget(buttons);
    connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::reject);
    connect(m_sources, &QListWidget::currentRowChanged, this, &SourcesDialog::selectSource);
    connect(m_filter, &QLineEdit::textChanged, this, &SourcesDialog::filterSources);
    connect(m_save, &QPushButton::clicked, this, &SourcesDialog::saveSource);
    connect(m_refresh, &QPushButton::clicked, this, [this]
    {
        m_status->setText(tr("Updating sources…"));
        m_refresh->setEnabled(false);
        m_request = Client::instance()->request(u"refresh"_s);
    });
    connect(m_connect, &QPushButton::clicked, this, [this]
    {
        const auto *browser = qobject_cast<QComboBox *>(m_fields.value(u"browser"_s));
        m_connect->setEnabled(false);
        m_status->setText(tr("Opening extension setup…"));
        m_connectRequest = Client::instance()->request(u"connectBrowser"_s,
            {{u"browser"_s, browser ? browser->currentData().toString() : u"auto"_s}});
    });
    connect(Client::instance(), &Client::eventReceived, this, &SourcesDialog::handleEvent);
    loadSources();
}

void Content::SourcesDialog::changeEvent(QEvent *event)
{
    QDialog::changeEvent(event);
    if ((event->type() == QEvent::ActivationChange) && isActiveWindow()
        && m_connect->isVisible() && !m_connectRequest)
    {
        m_connectRequest = Client::instance()->request(u"browserStatus"_s);
    }
}

void Content::SourcesDialog::loadSources()
{
    m_request = Client::instance()->request(u"sources"_s);
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
    m_save->setEnabled(row >= 0);
    m_enabled->setEnabled(row >= 0);
    const bool browser = (row >= 0) && (m_catalog.at(row).toObject().value(u"id"_s).toString() == u"browser");
    m_connect->setVisible(browser);
    m_browserSetup->setVisible(browser && !m_browserSetup->text().isEmpty());
    m_extensionDirectory->setVisible(browser && !m_extensionDirectory->text().isEmpty());
    if (row < 0)
    {
        m_name->clear();
        m_enabled->hide();
        return;
    }

    const QJsonObject source = m_catalog.at(row).toObject();
    m_name->setText(source.value(u"name"_s).toString());
    m_enabled->setVisible(!source.value(u"categories"_s).toArray().isEmpty());
    m_enabled->setChecked(source.value(u"enabled"_s).toBool());
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
                combo->addItem(option.value(u"label"_s).toString(), option.value(u"value"_s).toString());
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
        m_fields.insert(name, field);
        auto *label = new QLabel(setting.value(u"label"_s).toString(), this);
        label->setWordWrap(true);
        m_form->addRow(label, field);
    }
    if (browser && !m_connectRequest)
        m_connectRequest = Client::instance()->request(u"browserStatus"_s);
}

void Content::SourcesDialog::saveSource()
{
    const int row = m_sources->currentRow();
    if (row < 0)
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
    if (id == m_connectRequest)
    {
        if (event == u"browser")
        {
            const QJsonObject browser = data.toObject();
            const bool ready = browser.value(u"ready"_s).toBool();
            const QString directory = browser.value(u"extensionDirectory"_s).toString();
            if (ready)
            {
                m_browserSetup->clear();
                m_extensionDirectory->clear();
            }
            else if (!directory.isEmpty())
            {
                const QString browserId = browser.value(u"browser"_s).toString();
                m_browserSetup->setText(browserId == u"firefox"
                    ? tr("In Firefox, choose Load Temporary Add-on… and select manifest.json in this folder:")
                    : tr("Open %1 in the browser (Ctrl+L). Turn on Developer mode, choose Load unpacked, and select this folder:")
                        .arg(browserId == u"edge" ? u"edge://extensions"_s : u"chrome://extensions"_s));
                m_extensionDirectory->setText(directory);
            }
            const bool browserPage = !m_connect->isHidden();
            m_browserSetup->setVisible(browserPage && !m_browserSetup->text().isEmpty());
            m_extensionDirectory->setVisible(browserPage && !m_extensionDirectory->text().isEmpty());
            if (browserPage)
            {
                QString name;
                const QString connected = browser.value(u"connectedBrowser"_s).toString();
                for (const QJsonValue &value : browser.value(u"browsers"_s).toArray())
                {
                    const QJsonObject installed = value.toObject();
                    if (installed.value(u"id"_s).toString() == connected)
                        name = installed.value(u"name"_s).toString();
                }
                m_status->setText(ready
                    ? (name.isEmpty() ? tr("Connected") : tr("Connected to %1").arg(name))
                    : tr("Not connected"));
            }
        }
        if ((event == u"complete") || (event == u"error"))
        {
            m_connectRequest = 0;
            m_connect->setEnabled(true);
            if ((event == u"error") && !m_connect->isHidden())
                m_status->setText(data.toObject().value(u"message"_s).toString());
        }
        return;
    }
    if (id != m_request)
        return;

    if (event == u"sources")
    {
        const QString selected = m_name->text();
        m_catalog = {};
        for (const QString &sourceId : {u"browser"_s, u"ai"_s})
        {
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
            if ((sourceId != u"browser") && (sourceId != u"ai"))
                m_catalog.append(value);
        }
        m_sources->clear();
        for (const QJsonValue &value : m_catalog)
        {
            const QJsonObject source = value.toObject();
            auto *item = new QListWidgetItem(source.value(u"name"_s).toString(), m_sources);
            item->setToolTip(source.value(u"url"_s).toString());
        }
        const QList<QListWidgetItem *> matches = m_sources->findItems(selected, Qt::MatchExactly);
        m_sources->setCurrentItem(matches.isEmpty() ? m_sources->item(0) : matches.first());
        filterSources();
        m_status->setText(tr("%n source(s)", nullptr, m_catalog.size()));
    }
    else if (event == u"error")
    {
        m_status->setText(data.toObject().value(u"message"_s).toString());
        m_save->setEnabled(m_sources->currentRow() >= 0);
        m_refresh->setEnabled(true);
    }
    else if (event == u"complete")
    {
        m_save->setEnabled(m_sources->currentRow() >= 0);
        if (!m_refresh->isEnabled())
        {
            m_refresh->setEnabled(true);
            loadSources();
        }
        else if (m_status->text() == tr("Saving…"))
        {
            m_status->setText(tr("Saved"));
            loadSources();
        }
    }
}

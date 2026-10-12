// SPDX-License-Identifier: GPL-2.0-or-later
#include "translationsettingsdialog.h"

#include <QAction>
#include <QComboBox>
#include <QDialogButtonBox>
#include <QFormLayout>
#include <QJsonArray>
#include <QJsonObject>
#include <QLabel>
#include <QLineEdit>
#include <QPushButton>
#include <QVBoxLayout>

#include "base/content/client.h"
#include "base/global.h"
#include "gui/uithememanager.h"

Content::TranslationSettingsDialog::TranslationSettingsDialog(QWidget *parent)
    : QDialog(parent)
{
    setWindowTitle(tr("AI translation"));
    resize(560, 280);

    auto *layout = new QVBoxLayout(this);
    m_fields = new QWidget(this);
    auto *form = new QFormLayout(m_fields);
    form->setContentsMargins(0, 0, 0, 0);
    form->setFieldGrowthPolicy(QFormLayout::AllNonFixedFieldsGrow);
    m_provider = new QComboBox(m_fields);
    m_provider->addItem(tr("Automatic"), u"auto"_s);
    m_provider->addItem(u"Codex"_s, u"codex"_s);
    m_provider->addItem(u"OpenRouter"_s, u"openrouter"_s);
    m_provider->addItem(tr("OpenAI compatible"), u"openai-compatible"_s);
    m_provider->setCurrentIndex(-1);
    form->addRow(tr("Provider"), m_provider);

    m_apiKey = new QLineEdit(m_fields);
    m_apiKey->setEchoMode(QLineEdit::Password);
    QAction *clear = m_apiKey->addAction(UIThemeManager::instance()->getIcon(u"edit-clear"_s), QLineEdit::TrailingPosition);
    clear->setToolTip(tr("Clear saved value"));
    connect(clear, &QAction::triggered, this, [this]
    {
        m_apiKey->clear();
        m_apiKey->setModified(true);
        m_apiKey->setPlaceholderText(tr("Will be cleared"));
    });
    form->addRow(tr("API key"), m_apiKey);

    m_model = new QLineEdit(m_fields);
    form->addRow(tr("Model"), m_model);
    m_baseUrl = new QLineEdit(m_fields);
    form->addRow(tr("API address"), m_baseUrl);
    layout->addWidget(m_fields);
    m_fields->setEnabled(false);

    m_status = new QLabel(tr("Loading…"), this);
    m_status->setTextFormat(Qt::PlainText);
    m_status->setWordWrap(true);
    layout->addWidget(m_status);
    layout->addStretch();

    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Close, this);
    m_save = buttons->addButton(tr("Save"), QDialogButtonBox::ActionRole);
    m_save->setEnabled(false);
    layout->addWidget(buttons);
    connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::reject);
    connect(m_save, &QPushButton::clicked, this, &TranslationSettingsDialog::save);
    connect(Client::instance(), &Client::eventReceived, this, &TranslationSettingsDialog::handleEvent);
    m_request = Client::instance()->request(u"translationSettings"_s);
}

Content::TranslationSettingsDialog::~TranslationSettingsDialog()
{
    Client::instance()->cancel(m_request);
}

void Content::TranslationSettingsDialog::save()
{
    QJsonObject settings {
        {u"provider"_s, m_provider->currentData().toString()},
        {u"model"_s, m_model->text()},
        {u"baseUrl"_s, m_baseUrl->text()}
    };
    if (m_apiKey->isModified())
        settings.insert(u"apiKey"_s, m_apiKey->text());

    m_fields->setEnabled(false);
    m_save->setEnabled(false);
    m_status->setText(tr("Saving…"));
    m_request = Client::instance()->request(u"configureTranslation"_s, {{u"settings"_s, settings}});
}

void Content::TranslationSettingsDialog::handleEvent(const qint64 id, const QString &event, const QJsonValue &data)
{
    if (id != m_request)
        return;

    if (event == u"translationSettings")
    {
        for (const QJsonValue &value : data.toObject().value(u"settings"_s).toArray())
        {
            const QJsonObject setting = value.toObject();
            const QString name = setting.value(u"name"_s).toString();
            const QString content = setting.value(u"value"_s).toString();
            if (name == u"provider")
                m_provider->setCurrentIndex(m_provider->findData(content));
            else if (name == u"model")
                m_model->setText(content);
            else if (name == u"baseUrl")
                m_baseUrl->setText(content);
            else if (name == u"apiKey")
            {
                m_apiKey->clear();
                m_apiKey->setModified(false);
                m_apiKey->setPlaceholderText(setting.value(u"configured"_s).toBool() ? tr("Saved") : QString());
            }
        }
        m_status->clear();
    }
    else if (event == u"error")
    {
        m_status->setText(data.toObject().value(u"message"_s).toString());
    }

    if ((event == u"complete") || (event == u"error"))
    {
        m_request = 0;
        const bool loaded = m_provider->currentIndex() >= 0;
        m_fields->setEnabled(loaded);
        m_save->setEnabled(loaded);
    }
}

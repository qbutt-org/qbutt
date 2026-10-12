// SPDX-License-Identifier: GPL-2.0-or-later
#include "browserconnectionwidget.h"

#include <QComboBox>
#include <QHideEvent>
#include <QHBoxLayout>
#include <QJsonArray>
#include <QJsonObject>
#include <QLabel>
#include <QLineEdit>
#include <QProgressBar>
#include <QPushButton>
#include <QShowEvent>
#include <QSignalBlocker>
#include <QTimer>
#include <QVBoxLayout>

#include "base/content/client.h"
#include "base/global.h"
#include "base/preferences.h"

Content::BrowserConnectionWidget::BrowserConnectionWidget(const bool chooseBrowser, QWidget *parent)
    : QWidget(parent)
    , m_chooseBrowser {chooseBrowser}
{
    auto *layout = new QVBoxLayout(this);
    layout->setSizeConstraint(QLayout::SetMinimumSize);
    layout->setContentsMargins(0, 0, 0, 0);
    m_browserRow = new QWidget(this);
    auto *browserLayout = new QHBoxLayout(m_browserRow);
    browserLayout->setContentsMargins(0, 0, 0, 0);
    browserLayout->addWidget(new QLabel(tr("Browser"), m_browserRow));
    m_browser = new QComboBox(m_browserRow);
    m_browser->setObjectName(u"contentBrowserChoice"_s);
    m_browser->addItem(tr("Automatic"), u"auto"_s);
    m_browser->addItem(u"Microsoft Edge"_s, u"edge"_s);
    m_browser->addItem(u"Google Chrome"_s, u"chrome"_s);
    browserLayout->addWidget(m_browser, 1);
    m_browserRow->setVisible(m_chooseBrowser);
    layout->addWidget(m_browserRow);

    m_connect = new QPushButton(tr("Install extension"), this);
    m_connect->setObjectName(u"contentBrowserInstall"_s);
    m_connect->setEnabled(false);
    layout->addWidget(m_connect, 0, Qt::AlignLeft);
    m_progress = new QProgressBar(this);
    m_progress->setRange(0, 0);
    m_progress->setTextVisible(false);
    m_progress->setMaximumHeight(4);
    m_progress->hide();
    layout->addWidget(m_progress);
    m_status = new QLabel(this);
    m_status->setObjectName(u"contentBrowserStatus"_s);
    m_status->setTextFormat(Qt::PlainText);
    m_status->setWordWrap(true);
    layout->addWidget(m_status);
    m_setup = new QLabel(this);
    m_setup->setObjectName(u"contentBrowserSetup"_s);
    m_setup->setWordWrap(true);
    m_setup->hide();
    layout->addWidget(m_setup);
    m_extensionDirectory = new QLineEdit(this);
    m_extensionDirectory->setObjectName(u"contentBrowserExtensionDirectory"_s);
    m_extensionDirectory->setReadOnly(true);
    m_extensionDirectory->hide();
    layout->addWidget(m_extensionDirectory);

    m_poll = new QTimer(this);
    m_poll->setInterval(2000);
    connect(m_poll, &QTimer::timeout, this, &BrowserConnectionWidget::refreshStatus);
    connect(m_connect, &QPushButton::clicked, this, &BrowserConnectionWidget::connectBrowser);
    connect(m_browser, &QComboBox::currentIndexChanged, this, [this]
    {
        const QString choice = m_browser->currentData().toString();
        Preferences::instance()->setBrowserChoice(choice);
        Client::instance()->request(u"chooseBrowser"_s, {{u"browser"_s, choice}});
        m_poll->stop();
        Client::instance()->cancel(m_request);
        m_request = 0;
        m_ready = false;
        m_setup->hide();
        m_extensionDirectory->clear();
        m_extensionDirectory->hide();
        m_status->setText(tr("Checking connection…"));
        m_connect->setEnabled(false);
        m_connect->hide();
        m_poll->start();
        refreshStatus();
    });
    connect(Client::instance(), &Client::eventReceived, this, &BrowserConnectionWidget::handleEvent);
}

Content::BrowserConnectionWidget::~BrowserConnectionWidget()
{
    Client::instance()->cancel(m_request);
}

void Content::BrowserConnectionWidget::showEvent(QShowEvent *event)
{
    QWidget::showEvent(event);
    {
        const QSignalBlocker blocker(m_browser);
        m_browser->setCurrentIndex(m_browser->findData(Preferences::instance()->browserChoice()));
    }
    m_setup->hide();
    m_extensionDirectory->clear();
    m_extensionDirectory->hide();
    m_connect->setEnabled(false);
    m_status->setText(tr("Checking connection…"));
    m_poll->start();
    refreshStatus();
}

void Content::BrowserConnectionWidget::hideEvent(QHideEvent *event)
{
    m_poll->stop();
    Client::instance()->cancel(m_request);
    m_request = 0;
    m_connecting = false;
    m_ready = false;
    m_progress->hide();
    m_browser->setEnabled(true);
    QWidget::hideEvent(event);
}

void Content::BrowserConnectionWidget::refreshStatus()
{
    if (!m_request)
        m_request = Client::instance()->request(u"browserStatus"_s);
}

void Content::BrowserConnectionWidget::connectBrowser()
{
    if (m_request)
        return;

    m_poll->start();
    if (!m_extensionDirectory->text().isEmpty())
    {
        m_connect->setEnabled(false);
        m_status->setText(tr("Checking connection…"));
        refreshStatus();
        return;
    }

    m_connecting = true;
    m_browser->setEnabled(false);
    m_connect->setEnabled(false);
    m_status->setText(m_installed ? tr("Connecting extension…") : tr("Installing qcatch…"));
    m_progress->show();
    m_request = Client::instance()->request(u"connectBrowser"_s,
        {{u"browser"_s, m_browser->currentData().toString()}});
}

void Content::BrowserConnectionWidget::applyStatus(const QJsonObject &status)
{
    if (status.contains(u"qcatchInstalled"_s))
        m_installed = status.value(u"qcatchInstalled"_s).toBool();
    else if (m_connecting)
        m_installed = true;

    if (!m_installed)
    {
        m_setup->hide();
        m_extensionDirectory->clear();
        m_extensionDirectory->hide();
    }

    const bool wasReady = m_ready;
    m_ready = status.value(u"ready"_s).toBool();
    const bool browserInstalled = !status.value(u"browsers"_s).toArray().isEmpty();
    m_connect->setVisible(!m_ready && browserInstalled);
    m_connect->setText(!m_extensionDirectory->text().isEmpty() ? tr("Check connection")
        : (m_installed ? tr("Connect extension") : tr("Install extension")));
    m_connect->setEnabled(!m_connecting && browserInstalled);
    if (m_ready)
    {
        m_status->setText(tr("Connected"));
        m_setup->hide();
        m_extensionDirectory->clear();
        m_extensionDirectory->hide();
        if (!wasReady)
            emit connected();
        return;
    }

    const QString directory = status.value(u"extensionDirectory"_s).toString();
    if (!directory.isEmpty())
    {
        const QString browser = status.value(u"browser"_s).toString();
        m_setup->setText(tr("In %1, turn on Developer mode, choose Load unpacked, and select this folder:")
            .arg(browser == u"edge" ? u"edge://extensions"_s : u"chrome://extensions"_s));
        m_setup->show();
        m_extensionDirectory->setText(directory);
        m_extensionDirectory->show();
        m_connect->setText(tr("Check connection"));
    }
    if (!status.value(u"error"_s).toString().isEmpty())
        m_status->setText(tr("The browser connection is unavailable. Restart qbutt and try again."));
    else if (!browserInstalled)
        m_status->setText(tr("Install Microsoft Edge or Google Chrome to use qcatch."));
    else
        m_status->setText(m_extensionDirectory->text().isEmpty() ? QString() : tr("Waiting for the extension…"));
}

void Content::BrowserConnectionWidget::handleEvent(const qint64 id, const QString &event, const QJsonValue &data)
{
    if (id != m_request)
        return;

    if (event == u"browser")
        applyStatus(data.toObject());
    if ((event != u"complete") && (event != u"error"))
        return;

    m_request = 0;
    m_connecting = false;
    m_browser->setEnabled(true);
    m_progress->hide();
    m_connect->setEnabled(m_connect->isVisible());
    if (event == u"error")
    {
        m_poll->stop();
        m_status->setText(data.toObject().value(u"message"_s).toString());
        m_connect->setText(tr("Try again"));
        m_connect->setEnabled(true);
        m_connect->show();
    }
}

/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "pathswidget.h"

#ifdef Q_OS_WIN
#include <winsock2.h>
#include <ws2ipdef.h>
#include <iphlpapi.h>
#endif

#include <QCheckBox>
#include <QComboBox>
#include <QFileDialog>
#include <QFormLayout>
#include <QHBoxLayout>
#include <QJsonObject>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QNetworkInterface>
#include <QPushButton>
#include <QSignalBlocker>
#include <QVBoxLayout>

#include "base/global.h"
#include "base/net/pathmanager.h"
#include "base/net/proxyconfigurationmanager.h"

PathsWidget::PathsWidget(QWidget *parent)
    : QGroupBox(tr("Mihomo subscription (experimental)"), parent)
    , m_manager {Net::PathManager::instance()}
    , m_url {new QLineEdit(this)}
    , m_nodes {new QComboBox(this)}
    , m_interfaces {new QComboBox(this)}
    , m_mode {new QComboBox(this)}
    , m_includeNative {new QCheckBox(tr("Include Native through the selected physical interface"), this)}
    , m_paths {new QListWidget(this)}
    , m_refresh {new QPushButton(tr("Refresh"), this)}
    , m_localFile {new QPushButton(tr("Local file…"), this)}
    , m_start {new QPushButton(tr("Connect selected node"), this)}
    , m_disconnect {new QPushButton(tr("Disconnect selected path"), this)}
    , m_native {new QPushButton(tr("Use default connection"), this)}
    , m_status {new QLabel(this)}
{
    auto *layout = new QVBoxLayout(this);
    auto *form = new QFormLayout;
    auto *subscription = new QHBoxLayout;
    m_url->setObjectName(u"mihomoSubscriptionUrl"_s);
    m_url->setPlaceholderText(u"https://…"_s);
    m_url->setEchoMode(QLineEdit::PasswordEchoOnEdit);
    m_url->setText(m_manager->subscriptionUrl());
    m_url->setToolTip(tr("Stored only in your qbutt profile. Refresh uses the regular control network."));
    subscription->addWidget(m_url, 1);
    subscription->addWidget(m_refresh);
    subscription->addWidget(m_localFile);
    form->addRow(tr("Subscription:"), subscription);
    m_nodes->setObjectName(u"mihomoNode"_s);
    form->addRow(tr("Node:"), m_nodes);

    m_interfaces->setObjectName(u"mihomoPhysicalInterface"_s);
    m_interfaces->addItem(tr("Choose a physical interface"), QString());
    const QList<QNetworkInterface> interfaces = QNetworkInterface::allInterfaces();
    for (const QNetworkInterface &iface : interfaces)
    {
        if (!iface.flags().testFlag(QNetworkInterface::IsUp)
            || iface.flags().testFlag(QNetworkInterface::IsLoopBack)
            || ((iface.type() != QNetworkInterface::Ethernet) && (iface.type() != QNetworkInterface::Wifi)))
        {
            continue;
        }
#ifdef Q_OS_WIN
        MIB_IF_ROW2 row {};
        row.InterfaceIndex = static_cast<NET_IFINDEX>(iface.index());
        if ((GetIfEntry2(&row) != NO_ERROR) || !row.InterfaceAndOperStatusFlags.HardwareInterface)
            continue;
        const QString bindName = iface.humanReadableName();
#else
        const QString bindName = iface.name();
#endif
        m_interfaces->addItem(iface.humanReadableName(), bindName);
    }
    const int savedInterface = m_interfaces->findData(m_manager->interfaceName());
    if (savedInterface > 0)
        m_interfaces->setCurrentIndex(savedInterface);
    else if (m_interfaces->count() == 2)
        m_interfaces->setCurrentIndex(1);
    m_interfaces->setToolTip(tr("The selected adapter is bound by qbutt-net. Its actual route must still be verified."));
    form->addRow(tr("Interface:"), m_interfaces);
    m_mode->setObjectName(u"mihomoPeerPolicy"_s);
    m_mode->addItem(tr("Pinned TCP — first selected edge"), u"pinned"_s);
    m_mode->addItem(tr("Mixed TCP — selected edges"), u"mixed"_s);
    form->addRow(tr("Peer connections:"), m_mode);
    m_includeNative->setObjectName(u"mihomoIncludeNative"_s);
    form->addRow(QString(), m_includeNative);
    layout->addLayout(form);

    m_paths->setObjectName(u"mihomoPaths"_s);
    m_paths->setMaximumHeight(110);
    layout->addWidget(m_paths);

    auto *actions = new QHBoxLayout;
    m_start->setObjectName(u"startPinnedPath"_s);
    m_native->setObjectName(u"useNativePath"_s);
    actions->addWidget(m_start);
    actions->addWidget(m_disconnect);
    actions->addWidget(m_native);
    actions->addStretch();
    layout->addLayout(actions);
    auto *description = new QLabel(tr("Mixed shares one torrent session across selected TCP paths. Including Native exposes your "
        "home address to public torrent peers. Private torrents and trackers stay on the first tunnel edge. "
        "UDP, DHT and public inbound are not available in this build."), this);
    description->setWordWrap(true);
    layout->addWidget(description);
    m_status->setWordWrap(true);
    m_status->setTextFormat(Qt::PlainText);
    m_status->setObjectName(u"mihomoPathStatus"_s);
    layout->addWidget(m_status);

    connect(m_refresh, &QPushButton::clicked, this, [this]()
    {
        m_manager->refreshSubscription(m_url->text());
    });
    connect(m_localFile, &QPushButton::clicked, this, [this]()
    {
        const QString fileName = QFileDialog::getOpenFileName(this, tr("Select a Mihomo subscription"), {},
            tr("Mihomo YAML (*.yaml *.yml);;All files (*)"));
        if (!fileName.isEmpty())
            m_manager->inspectConfiguration(fileName);
    });
    connect(m_start, &QPushButton::clicked, this, [this]()
    {
        m_manager->openPath(m_manager->configurationPath(), m_nodes->currentData().toString(),
            m_interfaces->currentData().toString());
    });
    connect(m_disconnect, &QPushButton::clicked, this, [this]()
    {
        if (const auto *item = m_paths->currentItem())
            m_manager->stopPath(item->data(Qt::UserRole).toString());
    });
    const auto applyPolicy = [this]()
    {
        m_manager->setPolicy(m_mode->currentData().toString(),
            m_includeNative->isChecked() ? m_interfaces->currentData().toString() : QString());
    };
    connect(m_mode, &QComboBox::activated, this, applyPolicy);
    connect(m_includeNative, &QCheckBox::clicked, this, applyPolicy);
    connect(m_interfaces, &QComboBox::activated, this, [this]()
    {
        if (m_includeNative->isChecked())
            m_manager->setPolicy(m_mode->currentData().toString(), m_interfaces->currentData().toString());
    });
    connect(m_native, &QPushButton::clicked, m_manager, &Net::PathManager::useNative);
    connect(m_nodes, &QComboBox::currentIndexChanged, this, &PathsWidget::refreshState);
    connect(m_interfaces, &QComboBox::currentIndexChanged, this, &PathsWidget::refreshState);
    connect(m_paths, &QListWidget::currentRowChanged, this, &PathsWidget::refreshState);
    connect(m_manager, &Net::PathManager::changed, this, &PathsWidget::refreshState);
    connect(m_manager, &Net::PathManager::proxiesLoaded, this, [this](const QJsonArray &proxies)
    {
        const QString previous = (m_nodes->count() > 0) ? m_nodes->currentData().toString() : m_manager->proxyName();
        m_nodes->clear();
        for (const QJsonValue &value : proxies)
        {
            const QJsonObject node = value.toObject();
            const QString name = node.value(u"name"_s).toString();
            if (!name.isEmpty())
                m_nodes->addItem(u"%1 (%2)"_s.arg(name, node.value(u"type"_s).toString()), name);
        }
        const int previousIndex = m_nodes->findData(previous);
        if (previousIndex >= 0)
            m_nodes->setCurrentIndex(previousIndex);
        refreshState();
    });
    refreshState();
    if (!m_manager->configurationPath().isEmpty() && !m_manager->isBusy())
        m_manager->inspectConfiguration(m_manager->configurationPath());
}

void PathsWidget::refreshState()
{
    const bool busy = m_manager->isBusy();
    const QJsonObject state = m_manager->statusData();
    m_mode->setCurrentIndex(m_mode->findData(state.value(u"mode"_s).toString()));
    m_mode->setEnabled(!busy);
    m_includeNative->setChecked(!state.value(u"nativeInterface"_s).toString().isEmpty());
    m_includeNative->setEnabled(!busy && (m_mode->currentData() == u"mixed"_s)
        && !m_interfaces->currentData().toString().isEmpty());
    const QSignalBlocker pathsBlocker(m_paths);
    const QString selectedPath = m_paths->currentItem()
        ? m_paths->currentItem()->data(Qt::UserRole).toString() : QString();
    m_paths->clear();
    bool nodeConnected = false;
    for (const QJsonValue &value : state.value(u"paths"_s).toArray())
    {
        const QJsonObject path = value.toObject();
        const bool open = path.value(u"open"_s).toBool();
        const QString name = path.value(u"proxyName"_s).toString();
        auto *item = new QListWidgetItem(u"%1 — %2"_s.arg(name, open ? tr("Connected") : tr("Stopped")), m_paths);
        item->setData(Qt::UserRole, path.value(u"pathId"_s).toString());
        item->setData(Qt::UserRole + 1, open);
        if (item->data(Qt::UserRole).toString() == selectedPath)
            m_paths->setCurrentItem(item);
        nodeConnected |= open && (name == m_nodes->currentData().toString());
    }
    if (!m_paths->currentItem() && (m_paths->count() > 0))
        m_paths->setCurrentRow(0);
    m_disconnect->setEnabled(!busy && m_paths->currentItem()
        && m_paths->currentItem()->data(Qt::UserRole + 1).toBool());
    m_url->setEnabled(!busy);
    m_refresh->setEnabled(!busy);
    m_localFile->setEnabled(!busy);
    m_nodes->setEnabled(!busy);
    m_interfaces->setEnabled(!busy);
    m_start->setEnabled(!busy && !nodeConnected && (m_nodes->count() > 0)
        && !m_interfaces->currentData().toString().isEmpty());
    m_native->setEnabled(!busy && Net::ProxyConfigurationManager::instance()->hasRuntimeProxy());
    m_status->setText(busy ? tr("Working…") : m_manager->status());
}

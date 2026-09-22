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
#include <QSpinBox>
#include <QToolButton>
#include <QVBoxLayout>

#include "base/global.h"
#include "base/net/pathmanager.h"
#include "base/net/proxyconfigurationmanager.h"

PathsWidget::PathsWidget(QWidget *parent)
    : QGroupBox(tr("Mihomo subscription"), parent)
    , m_manager {Net::PathManager::instance()}
    , m_transportForm {new QFormLayout}
    , m_url {new QLineEdit(this)}
    , m_nodes {new QComboBox(this)}
    , m_sameServer {new QComboBox(this)}
    , m_groupServers {new QPushButton(tr("Group servers"), this)}
    , m_resetServerGroups {new QPushButton(tr("Reset grouping"), this)}
    , m_reserves {new QListWidget(this)}
    , m_interfaces {new QComboBox(this)}
    , m_mode {new QComboBox(this)}
    , m_dnsServer {new QLineEdit(this)}
    , m_bootstrapServer {new QLineEdit(this)}
    , m_dnsFamily {new QComboBox(this)}
    , m_dnsApply {new QPushButton(tr("Save DNS"), this)}
    , m_gatewayControlAddress {new QLineEdit(this)}
    , m_gatewayDatagramAddress {new QLineEdit(this)}
    , m_gatewayServerName {new QLineEdit(this)}
    , m_gatewayCaPath {new QLineEdit(this)}
    , m_gatewayCertificatePath {new QLineEdit(this)}
    , m_gatewayPrivateKeyPath {new QLineEdit(this)}
    , m_gatewayPort {new QSpinBox(this)}
    , m_gatewayTcp {new QCheckBox(tr("TCP"), this)}
    , m_gatewayUdp {new QCheckBox(tr("UDP / uTP / DHT"), this)}
    , m_gatewayApply {new QPushButton(tr("Save gateway"), this)}
    , m_paths {new QListWidget(this)}
    , m_refresh {new QPushButton(tr("Refresh"), this)}
    , m_localFile {new QPushButton(tr("Local file…"), this)}
    , m_start {new QPushButton(tr("Connect"), this)}
    , m_disconnect {new QPushButton(tr("Disconnect"), this)}
    , m_switch {new QPushButton(tr("Use selected backup"), this)}
    , m_native {new QPushButton(tr("Default connection"), this)}
    , m_status {new QLabel(this)}
{
    auto *layout = new QVBoxLayout(this);
    auto *form = new QFormLayout;
    auto *subscription = new QHBoxLayout;
    m_url->setObjectName(u"mihomoSubscriptionUrl"_s);
    m_localFile->setObjectName(u"mihomoLocalFile"_s);
    m_url->setPlaceholderText(u"https://…"_s);
    m_url->setEchoMode(QLineEdit::PasswordEchoOnEdit);
    m_url->setText(m_manager->subscriptionUrl());
    subscription->addWidget(m_url, 1);
    subscription->addWidget(m_refresh);
    subscription->addWidget(m_localFile);
    form->addRow(tr("Subscription:"), subscription);
    m_nodes->setObjectName(u"mihomoNode"_s);
    form->addRow(tr("Node:"), m_nodes);
    m_interfaces->setObjectName(u"mihomoPhysicalInterface"_s);
    m_interfaces->addItem(tr("Choose a network adapter"), QString());
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
    form->addRow(tr("Network adapter:"), m_interfaces);
    m_mode->setObjectName(u"mihomoPeerPolicy"_s);
    m_mode->addItem(tr("Single node"), u"pinned"_s);
    m_mode->addItem(tr("Selected nodes only"), u"tunnels"_s);
    m_mode->addItem(tr("Selected nodes + direct connection"), u"mixed"_s);
    m_mode->setItemData(0, tr("Use the first node in the connection list."), Qt::ToolTipRole);
    m_mode->setItemData(1, tr("Use connected nodes together. Private torrents use the first node in the list."), Qt::ToolTipRole);
    m_mode->setItemData(2, tr("Also use your direct connection, exposing its address to peers. Private torrents use the first node in the list."), Qt::ToolTipRole);
    form->addRow(tr("Mode:"), m_mode);
    layout->addLayout(form);

    m_paths->setObjectName(u"mihomoPaths"_s);
    m_paths->setMaximumHeight(110);
    layout->addWidget(m_paths);

    auto *actions = new QHBoxLayout;
    m_start->setObjectName(u"mihomoStart"_s);
    m_native->setObjectName(u"mihomoNative"_s);
    actions->addWidget(m_start);
    actions->addWidget(m_disconnect);
    actions->addWidget(m_native);
    actions->addStretch();
    layout->addLayout(actions);
    m_status->setWordWrap(true);
    m_status->setTextFormat(Qt::PlainText);
    m_status->setObjectName(u"mihomoPathStatus"_s);
    layout->addWidget(m_status);

    auto *advancedToggle = new QToolButton(this);
    advancedToggle->setObjectName(u"mihomoAdvancedSettings"_s);
    advancedToggle->setText(tr("Advanced"));
    advancedToggle->setToolButtonStyle(Qt::ToolButtonTextBesideIcon);
    advancedToggle->setArrowType(Qt::RightArrow);
    advancedToggle->setCheckable(true);
    auto *advancedOptions = new QWidget(this);
    advancedOptions->setObjectName(u"mihomoAdvancedOptions"_s);
    auto *advancedLayout = new QVBoxLayout(advancedOptions);
    advancedLayout->setContentsMargins(0, 0, 0, 0);
    auto *serverGrouping = new QHBoxLayout;
    m_sameServer->setObjectName(u"mihomoSameServer"_s);
    m_groupServers->setObjectName(u"mihomoGroupServers"_s);
    m_resetServerGroups->setObjectName(u"mihomoResetServerGroups"_s);
    m_groupServers->setToolTip(tr("Group only nodes you know share one server. Disconnect them first."));
    m_resetServerGroups->setToolTip(tr("Remove manual server groups."));
    serverGrouping->addWidget(m_sameServer, 1);
    serverGrouping->addWidget(m_groupServers);
    serverGrouping->addWidget(m_resetServerGroups);
    m_transportForm->addRow(tr("Same server as:"), serverGrouping);
    m_reserves->setObjectName(u"mihomoReserveTransports"_s);
    m_reserves->setMaximumHeight(75);
    m_reserves->setToolTip(tr("Choose up to three backup connections to the same server."));
    m_transportForm->addRow(tr("Backup connections:"), m_reserves);
    m_switch->setObjectName(u"mihomoSwitchTransport"_s);
    m_transportForm->addRow(QString(), m_switch);
    advancedLayout->addLayout(m_transportForm);
    advancedOptions->hide();
    layout->addWidget(advancedToggle, 0, Qt::AlignLeft);
    layout->addWidget(advancedOptions);
    connect(advancedToggle, &QToolButton::toggled, this, [advancedToggle, advancedOptions](const bool expanded)
    {
        advancedToggle->setArrowType(expanded ? Qt::DownArrow : Qt::RightArrow);
        advancedOptions->setVisible(expanded);
    });

    auto *dnsForm = new QFormLayout;
    m_dnsServer->setObjectName(u"mihomoDnsServer"_s);
    m_bootstrapServer->setObjectName(u"mihomoBootstrapServer"_s);
    m_dnsFamily->setObjectName(u"mihomoDnsFamily"_s);
    m_dnsApply->setObjectName(u"mihomoSaveDns"_s);
    m_dnsServer->setToolTip(tr("IP:port. Resolve torrent addresses through the connected node."));
    m_bootstrapServer->setToolTip(tr("IP:port. Resolve the node's own address through the network adapter."));
    m_dnsApply->setToolTip(tr("Applies when a node reconnects."));
    m_dnsFamily->addItem(tr("IPv4 and IPv6"), u"dual"_s);
    m_dnsFamily->addItem(tr("IPv4 only"), u"ipv4"_s);
    m_dnsFamily->addItem(tr("IPv6 only"), u"ipv6"_s);
    dnsForm->addRow(tr("DNS server:"), m_dnsServer);
    dnsForm->addRow(tr("Bootstrap DNS:"), m_bootstrapServer);
    dnsForm->addRow(tr("Destination addresses:"), m_dnsFamily);
    dnsForm->addRow(QString(), m_dnsApply);
    advancedLayout->addLayout(dnsForm);
    const auto loadDnsSettings = [this]()
    {
        const QJsonObject dns = m_manager->dnsPolicy();
        m_dnsServer->setText(dns.value(u"server"_s).toString());
        m_bootstrapServer->setText(dns.value(u"bootstrapServer"_s).toString());
        m_dnsFamily->setCurrentIndex(m_dnsFamily->findData(dns.value(u"family"_s).toString()));
    };
    loadDnsSettings();
    connect(m_manager, &Net::PathManager::dnsPolicyChanged, this, loadDnsSettings);
    connect(m_dnsApply, &QPushButton::clicked, this, [this]()
    {
        m_manager->setDnsPolicy(m_dnsServer->text(), m_bootstrapServer->text(), m_dnsFamily->currentData().toString());
    });

    advancedLayout->addWidget(new QLabel(tr("Incoming connections"), advancedOptions));
    auto *gatewayForm = new QFormLayout;
    m_gatewayControlAddress->setObjectName(u"mihomoGatewayControlAddress"_s);
    m_gatewayDatagramAddress->setObjectName(u"mihomoGatewayDatagramAddress"_s);
    m_gatewayServerName->setObjectName(u"mihomoGatewayServerName"_s);
    m_gatewayCaPath->setObjectName(u"mihomoGatewayCaPath"_s);
    m_gatewayCertificatePath->setObjectName(u"mihomoGatewayCertificatePath"_s);
    m_gatewayPrivateKeyPath->setObjectName(u"mihomoGatewayPrivateKeyPath"_s);
    m_gatewayPort->setObjectName(u"mihomoGatewayPort"_s);
    m_gatewayTcp->setObjectName(u"mihomoGatewayTcp"_s);
    m_gatewayUdp->setObjectName(u"mihomoGatewayUdp"_s);
    m_gatewayApply->setObjectName(u"mihomoSaveGateway"_s);
    m_gatewayControlAddress->setPlaceholderText(u"gateway.example:443"_s);
    m_gatewayDatagramAddress->setPlaceholderText(u"gateway.example:443"_s);
    m_gatewayServerName->setPlaceholderText(u"gateway.example"_s);
    m_gatewayPort->setRange(0, 65535);
    m_gatewayPort->setSpecialValueText(tr("Automatic"));
    auto *protocols = new QHBoxLayout;
    protocols->addWidget(m_gatewayTcp);
    protocols->addWidget(m_gatewayUdp);
    protocols->addStretch();
    gatewayForm->addRow(tr("Control endpoint:"), m_gatewayControlAddress);
    gatewayForm->addRow(tr("Datagram endpoint:"), m_gatewayDatagramAddress);
    gatewayForm->addRow(tr("TLS server name:"), m_gatewayServerName);
    gatewayForm->addRow(tr("CA certificate:"), m_gatewayCaPath);
    gatewayForm->addRow(tr("Client certificate:"), m_gatewayCertificatePath);
    gatewayForm->addRow(tr("Client private key:"), m_gatewayPrivateKeyPath);
    gatewayForm->addRow(tr("Requested port:"), m_gatewayPort);
    gatewayForm->addRow(tr("Listeners:"), protocols);
    gatewayForm->addRow(QString(), m_gatewayApply);
    m_gatewayTcp->setToolTip(tr("Receive connections through your public gateway."));
    m_gatewayUdp->setToolTip(tr("Receive connections through your public gateway."));
    advancedLayout->addLayout(gatewayForm);
    const QJsonObject gateway = m_manager->gatewayConfiguration();
    m_gatewayControlAddress->setText(gateway.value(u"controlAddress"_s).toString());
    m_gatewayDatagramAddress->setText(gateway.value(u"datagramAddress"_s).toString());
    m_gatewayServerName->setText(gateway.value(u"serverName"_s).toString());
    m_gatewayCaPath->setText(gateway.value(u"caPath"_s).toString());
    m_gatewayCertificatePath->setText(gateway.value(u"certificatePath"_s).toString());
    m_gatewayPrivateKeyPath->setText(gateway.value(u"privateKeyPath"_s).toString());
    m_gatewayPort->setValue(gateway.value(u"port"_s).toInt());
    m_gatewayTcp->setChecked(gateway.value(u"tcp"_s).toBool());
    m_gatewayUdp->setChecked(gateway.value(u"udp"_s).toBool());
    connect(m_gatewayUdp, &QCheckBox::toggled, m_gatewayDatagramAddress, &QWidget::setEnabled);
    m_gatewayDatagramAddress->setEnabled(m_gatewayUdp->isChecked());
    connect(m_gatewayApply, &QPushButton::clicked, this, [this]()
    {
        m_manager->setGatewayConfiguration({
            {u"controlAddress"_s, m_gatewayControlAddress->text()},
            {u"datagramAddress"_s, m_gatewayDatagramAddress->text()},
            {u"serverName"_s, m_gatewayServerName->text()},
            {u"caPath"_s, m_gatewayCaPath->text()},
            {u"certificatePath"_s, m_gatewayCertificatePath->text()},
            {u"privateKeyPath"_s, m_gatewayPrivateKeyPath->text()},
            {u"port"_s, m_gatewayPort->value()},
            {u"tcp"_s, m_gatewayTcp->isChecked()},
            {u"udp"_s, m_gatewayUdp->isChecked()}});
    });

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
        QStringList reserves;
        for (int row = 0; row < m_reserves->count(); ++row)
        {
            const QListWidgetItem *item = m_reserves->item(row);
            if (item->checkState() == Qt::Checked)
                reserves.append(item->data(Qt::UserRole).toString());
        }
        m_manager->openPath(m_manager->configurationPath(), m_nodes->currentData().toString(),
            m_interfaces->currentData().toString(), reserves);
    });
    connect(m_groupServers, &QPushButton::clicked, this, [this]()
    {
        m_manager->groupServers(m_nodes->currentData().toString(), m_sameServer->currentData().toString());
    });
    connect(m_resetServerGroups, &QPushButton::clicked, m_manager, &Net::PathManager::resetServerGroups);
    connect(m_sameServer, &QComboBox::currentIndexChanged, this, &PathsWidget::refreshState);
    connect(m_switch, &QPushButton::clicked, this, [this]()
    {
        if (m_paths->currentItem() && m_reserves->currentItem())
            m_manager->switchTransport(m_paths->currentItem()->data(Qt::UserRole).toString(),
                m_reserves->currentItem()->data(Qt::UserRole).toString());
    });
    connect(m_disconnect, &QPushButton::clicked, this, [this]()
    {
        if (const auto *item = m_paths->currentItem())
            m_manager->stopPath(item->data(Qt::UserRole).toString());
    });
    const auto applyPolicy = [this]()
    {
        m_manager->setPolicy(m_mode->currentData().toString(),
            (m_mode->currentData() == u"mixed"_s) ? m_interfaces->currentData().toString() : QString());
    };
    connect(m_mode, &QComboBox::activated, this, applyPolicy);
    connect(m_interfaces, &QComboBox::activated, this, [this]()
    {
        if (m_mode->currentData() == u"mixed"_s)
            m_manager->setPolicy(m_mode->currentData().toString(), m_interfaces->currentData().toString());
    });
    connect(m_native, &QPushButton::clicked, m_manager, &Net::PathManager::useNative);
    connect(m_nodes, &QComboBox::currentIndexChanged, this, &PathsWidget::refreshState);
    connect(m_nodes, &QComboBox::currentIndexChanged, this, &PathsWidget::refreshReserves);
    connect(m_interfaces, &QComboBox::currentIndexChanged, this, &PathsWidget::refreshState);
    connect(m_paths, &QListWidget::currentRowChanged, this, &PathsWidget::refreshState);
    connect(m_reserves, &QListWidget::currentRowChanged, this, &PathsWidget::refreshState);
    connect(m_reserves, &QListWidget::itemChanged, this, &PathsWidget::refreshState);
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
            {
                m_nodes->addItem(u"%1 (%2)"_s.arg(name, node.value(u"type"_s).toString()), name);
                m_nodes->setItemData(m_nodes->count() - 1, node.value(u"configuredServerId"_s).toString(), Qt::UserRole + 1);
            }
        }
        const int previousIndex = m_nodes->findData(previous);
        if (previousIndex >= 0)
            m_nodes->setCurrentIndex(previousIndex);
        refreshReserves();
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
    bool managedOpen = false;
    for (const QJsonValue &value : state.value(u"paths"_s).toArray())
    {
        const QJsonObject path = value.toObject();
        managedOpen = managedOpen || ((path.value(u"edgeId"_s) != u"native"_s) && path.value(u"open"_s).toBool());
    }
    m_sameServer->setEnabled(!busy && !managedOpen);
    m_groupServers->setEnabled(!busy && !managedOpen && !m_sameServer->currentData().toString().isEmpty());
    m_resetServerGroups->setEnabled(!busy && !managedOpen && !state.value(u"serverGroups"_s).toObject().isEmpty());
    m_mode->setCurrentIndex(m_mode->findData(state.value(u"mode"_s).toString()));
    m_mode->setEnabled(!busy);
    const QSignalBlocker pathsBlocker(m_paths);
    const QString selectedPath = m_paths->currentItem()
        ? m_paths->currentItem()->data(Qt::UserRole).toString() : QString();
    m_paths->clear();
    bool nodeConnected = false;
    for (const QJsonValue &value : state.value(u"paths"_s).toArray())
    {
        const QJsonObject path = value.toObject();
        const bool open = path.value(u"open"_s).toBool();
        const bool native = path.value(u"edgeId"_s) == u"native"_s;
        const QString name = path.value(u"proxyName"_s).toString();
        const QJsonObject gateway = path.value(u"gateway"_s).toObject();
        const QString publicEndpoint = gateway.value(u"publicEndpoint"_s).toString();
        QString pathState;
        if (!open)
            pathState = tr("Stopped");
        else if (native)
            pathState = tr("Connected");
        else
            pathState = publicEndpoint.isEmpty() ? tr("Connected, outgoing only") : tr("Connected, public %1").arg(publicEndpoint);
        const QString transport = path.value(u"transport"_s).toObject().value(u"state"_s).toString();
        if (open && (transport == u"checking"))
            pathState += tr("; checking backup connections");
        else if (open && (transport == u"unavailable"))
            pathState += tr("; no reachable backup found");
        else if (open && (transport == u"config-changed"))
            pathState += tr("; settings changed, reconnect to apply");
        auto *item = new QListWidgetItem(u"%1 — %2"_s.arg(native ? tr("Direct connection") : name, pathState), m_paths);
        item->setData(Qt::UserRole, path.value(u"pathId"_s).toString());
        item->setData(Qt::UserRole + 1, open);
        if (item->data(Qt::UserRole).toString() == selectedPath)
            m_paths->setCurrentItem(item);
        nodeConnected |= open && (name == m_nodes->currentData().toString());
    }
    if (!m_paths->currentItem() && (m_paths->count() > 0))
        m_paths->setCurrentRow(0);
    m_paths->setVisible(m_paths->count() > 0);
    m_disconnect->setVisible(m_paths->count() > 0);
    const bool resolving = state.value(u"resolution"_s).toObject().value(u"state"_s) == u"pending"_s;
    m_disconnect->setEnabled((!busy || resolving) && m_paths->currentItem()
        && m_paths->currentItem()->data(Qt::UserRole + 1).toBool());
    bool selectedReserve = false;
    const QListWidgetItem *reserve = m_reserves->currentItem();
    if (m_paths->currentItem() && reserve && (reserve->checkState() == Qt::Checked))
    {
        for (const QJsonValue &value : state.value(u"paths"_s).toArray())
        {
            const QJsonObject path = value.toObject();
            if (path.value(u"pathId"_s).toString() == m_paths->currentItem()->data(Qt::UserRole).toString())
                selectedReserve = path.value(u"open"_s).toBool()
                    && path.value(u"reserveNames"_s).toArray().contains(reserve->data(Qt::UserRole).toString());
        }
    }
    m_switch->setEnabled(!busy && selectedReserve);
    m_transportForm->setRowVisible(m_reserves, m_reserves->count() > 0);
    m_transportForm->setRowVisible(m_switch, m_reserves->count() > 0);
    m_url->setEnabled(!busy);
    m_refresh->setEnabled(!busy);
    m_localFile->setEnabled(!busy);
    m_nodes->setEnabled(!busy);
    m_reserves->setEnabled(!busy);
    m_interfaces->setEnabled(!busy);
    m_dnsServer->setEnabled(!busy);
    m_bootstrapServer->setEnabled(!busy);
    m_dnsFamily->setEnabled(!busy);
    m_dnsApply->setEnabled(!busy);
    m_gatewayControlAddress->setEnabled(!busy);
    m_gatewayDatagramAddress->setEnabled(!busy && m_gatewayUdp->isChecked());
    m_gatewayServerName->setEnabled(!busy);
    m_gatewayCaPath->setEnabled(!busy);
    m_gatewayCertificatePath->setEnabled(!busy);
    m_gatewayPrivateKeyPath->setEnabled(!busy);
    m_gatewayPort->setEnabled(!busy);
    m_gatewayTcp->setEnabled(!busy);
    m_gatewayUdp->setEnabled(!busy);
    m_gatewayApply->setEnabled(!busy);
    m_start->setEnabled(!busy && !nodeConnected && (m_nodes->count() > 0)
        && !m_interfaces->currentData().toString().isEmpty());
    m_native->setEnabled(!busy && Net::ProxyConfigurationManager::instance()->hasRuntimeProxy());
    m_status->setText(busy ? tr("Working…") : m_manager->status());
}

void PathsWidget::refreshReserves()
{
    const QString node = m_nodes->currentData().toString();
    QStringList selected = m_manager->reserveNames(node);
    if ((m_reserveNode == node) && (m_reserves->count() > 0))
    {
        selected.clear();
        for (int row = 0; row < m_reserves->count(); ++row)
        {
            if (m_reserves->item(row)->checkState() == Qt::Checked)
                selected.append(m_reserves->item(row)->data(Qt::UserRole).toString());
        }
    }
    m_reserveNode = node;
    m_reserves->clear();
    const QSignalBlocker groupingBlocker(m_sameServer);
    const QString previousTarget = m_sameServer->currentData().toString();
    m_sameServer->clear();
    m_sameServer->addItem(tr("Choose a node on the same server"), QString());
    const QString serverId = m_nodes->currentData(Qt::UserRole + 1).toString();
    if (serverId.isEmpty())
    {
        refreshState();
        return;
    }
    const QString edge = m_manager->edgeIdForServer(serverId);
    for (int index = 0; index < m_nodes->count(); ++index)
    {
        if (index == m_nodes->currentIndex())
            continue;
        const QString name = m_nodes->itemData(index).toString();
        const QString candidateId = m_nodes->itemData(index, Qt::UserRole + 1).toString();
        if (candidateId.isEmpty())
            continue;
        if (m_manager->edgeIdForServer(candidateId) != edge)
        {
            m_sameServer->addItem(m_nodes->itemText(index), name);
            continue;
        }
        auto *item = new QListWidgetItem(m_nodes->itemText(index), m_reserves);
        item->setData(Qt::UserRole, name);
        item->setFlags(item->flags() | Qt::ItemIsUserCheckable);
        item->setCheckState(selected.contains(name) ? Qt::Checked : Qt::Unchecked);
    }
    const int previousIndex = m_sameServer->findData(previousTarget);
    if (previousIndex >= 0)
        m_sameServer->setCurrentIndex(previousIndex);
    refreshState();
}

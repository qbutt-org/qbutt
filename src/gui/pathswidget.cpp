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
    , m_dnsServer {new QLineEdit(this)}
    , m_bootstrapServer {new QLineEdit(this)}
    , m_dnsFamily {new QComboBox(this)}
    , m_dnsApply {new QPushButton(tr("Save DNS settings"), this)}
    , m_gatewayControlAddress {new QLineEdit(this)}
    , m_gatewayDatagramAddress {new QLineEdit(this)}
    , m_gatewayServerName {new QLineEdit(this)}
    , m_gatewayCaPath {new QLineEdit(this)}
    , m_gatewayCertificatePath {new QLineEdit(this)}
    , m_gatewayPrivateKeyPath {new QLineEdit(this)}
    , m_gatewayPort {new QSpinBox(this)}
    , m_gatewayTcp {new QCheckBox(tr("TCP"), this)}
    , m_gatewayUdp {new QCheckBox(tr("UDP / uTP / DHT"), this)}
    , m_gatewayApply {new QPushButton(tr("Save public gateway"), this)}
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
    m_localFile->setObjectName(u"mihomoLocalFile"_s);
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
    m_mode->addItem(tr("Pinned — first selected edge"), u"pinned"_s);
    m_mode->addItem(tr("Tunnels only — selected remote edges"), u"tunnels"_s);
    m_mode->addItem(tr("Mixed — remote edges and Native"), u"mixed"_s);
    form->addRow(tr("Peer connections:"), m_mode);
    layout->addLayout(form);

    auto *dnsToggle = new QPushButton(tr("DNS settings…"), this);
    dnsToggle->setObjectName(u"mihomoDnsSettings"_s);
    dnsToggle->setCheckable(true);
    layout->addWidget(dnsToggle, 0, Qt::AlignLeft);
    auto *dnsOptions = new QWidget(this);
    auto *dnsForm = new QFormLayout(dnsOptions);
    dnsForm->setContentsMargins(0, 0, 0, 0);
    m_dnsServer->setObjectName(u"mihomoDnsServer"_s);
    m_bootstrapServer->setObjectName(u"mihomoBootstrapServer"_s);
    m_dnsFamily->setObjectName(u"mihomoDnsFamily"_s);
    m_dnsApply->setObjectName(u"mihomoSaveDns"_s);
    m_dnsServer->setToolTip(tr("Numeric IP:port. Hostname lookups use this DNS server through each selected node."));
    m_bootstrapServer->setToolTip(tr("Numeric IP:port. Only the node's own hostname is resolved through the selected physical interface."));
    m_dnsFamily->addItem(tr("IPv4 and IPv6"), u"dual"_s);
    m_dnsFamily->addItem(tr("IPv4 only"), u"ipv4"_s);
    m_dnsFamily->addItem(tr("IPv6 only"), u"ipv6"_s);
    dnsForm->addRow(tr("DNS server:"), m_dnsServer);
    dnsForm->addRow(tr("Bootstrap DNS:"), m_bootstrapServer);
    dnsForm->addRow(tr("Destination addresses:"), m_dnsFamily);
    dnsForm->addRow(QString(), m_dnsApply);
    auto *dnsDescription = new QLabel(tr("The default is Cloudflare DNS (1.1.1.1). Changes apply when connecting a node. "
        "Full application DNS isolation has not been verified."), dnsOptions);
    dnsDescription->setWordWrap(true);
    dnsForm->addRow(dnsDescription);
    dnsOptions->hide();
    layout->addWidget(dnsOptions);
    connect(dnsToggle, &QPushButton::toggled, dnsOptions, &QWidget::setVisible);
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

    auto *gatewayToggle = new QPushButton(tr("Public gateway settings…"), this);
    gatewayToggle->setObjectName(u"mihomoGatewaySettings"_s);
    gatewayToggle->setCheckable(true);
    layout->addWidget(gatewayToggle, 0, Qt::AlignLeft);
    auto *gatewayOptions = new QWidget(this);
    auto *gatewayForm = new QFormLayout(gatewayOptions);
    gatewayForm->setContentsMargins(0, 0, 0, 0);
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
    auto *protocols = new QWidget(gatewayOptions);
    auto *protocolsLayout = new QHBoxLayout(protocols);
    protocolsLayout->setContentsMargins(0, 0, 0, 0);
    protocolsLayout->addWidget(m_gatewayTcp);
    protocolsLayout->addWidget(m_gatewayUdp);
    protocolsLayout->addStretch();
    gatewayForm->addRow(tr("Control endpoint:"), m_gatewayControlAddress);
    gatewayForm->addRow(tr("Datagram endpoint:"), m_gatewayDatagramAddress);
    gatewayForm->addRow(tr("TLS server name:"), m_gatewayServerName);
    gatewayForm->addRow(tr("CA certificate:"), m_gatewayCaPath);
    gatewayForm->addRow(tr("Client certificate:"), m_gatewayCertificatePath);
    gatewayForm->addRow(tr("Client private key:"), m_gatewayPrivateKeyPath);
    gatewayForm->addRow(tr("Requested port:"), m_gatewayPort);
    gatewayForm->addRow(tr("Listeners:"), protocols);
    gatewayForm->addRow(QString(), m_gatewayApply);
    auto *gatewayDescription = new QLabel(tr("The gateway is optional. qbutt advertises a public endpoint only while its "
        "authenticated listener lease is active. Disabling both listeners retires existing leases."), gatewayOptions);
    gatewayDescription->setWordWrap(true);
    gatewayForm->addRow(gatewayDescription);
    gatewayOptions->hide();
    layout->addWidget(gatewayOptions);
    connect(gatewayToggle, &QPushButton::toggled, gatewayOptions, &QWidget::setVisible);
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
    auto *description = new QLabel(tr("All policies share one torrent session. Supported UDP routes carry uTP, UDP trackers and DHT. "
        "Public announces use active gateway leases. "
        "Including Native exposes its address to public torrent peers; private torrents stay on the first remote edge."), this);
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
        const QJsonObject gateway = path.value(u"gateway"_s).toObject();
        const QString publicEndpoint = gateway.value(u"publicEndpoint"_s).toString();
        const QString pathState = !open ? tr("Stopped") : (publicEndpoint.isEmpty()
            ? tr("Connected, outgoing only") : tr("Connected, public %1").arg(publicEndpoint));
        auto *item = new QListWidgetItem(u"%1 — %2"_s.arg(name, pathState), m_paths);
        item->setData(Qt::UserRole, path.value(u"pathId"_s).toString());
        item->setData(Qt::UserRole + 1, open);
        if (item->data(Qt::UserRole).toString() == selectedPath)
            m_paths->setCurrentItem(item);
        nodeConnected |= open && (name == m_nodes->currentData().toString());
    }
    if (!m_paths->currentItem() && (m_paths->count() > 0))
        m_paths->setCurrentRow(0);
    const bool resolving = state.value(u"resolution"_s).toObject().value(u"state"_s) == u"pending"_s;
    m_disconnect->setEnabled((!busy || resolving) && m_paths->currentItem()
        && m_paths->currentItem()->data(Qt::UserRole + 1).toBool());
    m_url->setEnabled(!busy);
    m_refresh->setEnabled(!busy);
    m_localFile->setEnabled(!busy);
    m_nodes->setEnabled(!busy);
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

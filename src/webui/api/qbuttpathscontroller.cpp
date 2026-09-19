/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qbuttpathscontroller.h"

#include "base/global.h"
#include "base/net/pathmanager.h"
#include "apierror.h"

void QbuttPathsController::statusAction()
{
    const auto *manager = Net::PathManager::instance();
    setResult(manager->statusData(true));
    if (manager->isBusy())
        setStatus(APIStatus::Async);
}

void QbuttPathsController::requireIdle() const
{
    if (Net::PathManager::instance()->isBusy())
        throw APIError(APIErrorType::Conflict, tr("A path operation is already running."));
}

void QbuttPathsController::listAction()
{
    requireParams({u"configPath"_s});
    requireIdle();
    Net::PathManager::instance()->inspectConfiguration(params().value(u"configPath"_s));
    statusAction();
}

void QbuttPathsController::refreshAction()
{
    requireParams({u"url"_s});
    requireIdle();
    Net::PathManager::instance()->refreshSubscription(params().value(u"url"_s));
    statusAction();
}

void QbuttPathsController::openAction()
{
    requireParams({u"configPath"_s, u"proxyName"_s, u"interfaceName"_s});
    requireIdle();
    Net::PathManager::instance()->openPath(params().value(u"configPath"_s),
        params().value(u"proxyName"_s), params().value(u"interfaceName"_s), params().value(u"edgeId"_s));
    statusAction();
}

void QbuttPathsController::policyAction()
{
    requireParams({u"mode"_s});
    requireIdle();
    const QString mode = params().value(u"mode"_s);
    if ((mode != u"mixed") && (mode != u"pinned") && (mode != u"tunnels"))
        throw APIError(APIErrorType::BadParams, tr("Unsupported network policy."));
    if ((mode == u"mixed") && params().value(u"nativeInterface"_s).isEmpty())
        throw APIError(APIErrorType::BadParams, tr("Mixed mode requires a physical Native interface."));
    auto *manager = Net::PathManager::instance();
    if (!manager->setPolicy(mode, params().value(u"nativeInterface"_s)))
        throw APIError(APIErrorType::BadParams, manager->status());
    statusAction();
}

void QbuttPathsController::stopAction()
{
    const QString pathIdText = params().value(u"pathId"_s);
    if (!pathIdText.isEmpty())
    {
        bool validPathId = false;
        const quint64 pathId = pathIdText.toULongLong(&validPathId);
        if (!validPathId || (pathId == 0) || (pathId > 9007199254740991)
            || (QString::number(pathId) != pathIdText))
        {
            throw APIError(APIErrorType::BadParams, tr("Invalid path identity."));
        }
    }
    const QJsonObject state = Net::PathManager::instance()->statusData();
    if (!pathIdText.isEmpty()
        && (state.value(u"resolution"_s).toObject().value(u"state"_s) != u"pending"_s))
    {
        requireIdle();
    }
    Net::PathManager::instance()->stopPath(pathIdText);
    statusAction();
}

void QbuttPathsController::dnsAction()
{
    requireParams({u"server"_s, u"bootstrapServer"_s, u"family"_s});
    requireIdle();
    auto *manager = Net::PathManager::instance();
    if (!manager->setDnsPolicy(params().value(u"server"_s), params().value(u"bootstrapServer"_s),
        params().value(u"family"_s)))
    {
        throw APIError(APIErrorType::BadParams, manager->status());
    }
    statusAction();
}

void QbuttPathsController::gatewayAction()
{
    requireParams({u"controlAddress"_s, u"datagramAddress"_s, u"serverName"_s,
        u"caPath"_s, u"certificatePath"_s, u"privateKeyPath"_s, u"port"_s, u"tcp"_s, u"udp"_s});
    requireIdle();
    const QString tcpText = params().value(u"tcp"_s);
    const QString udpText = params().value(u"udp"_s);
    bool validPort = false;
    const int port = params().value(u"port"_s).toInt(&validPort);
    if (!validPort || ((tcpText != u"true") && (tcpText != u"false"))
        || ((udpText != u"true") && (udpText != u"false")))
    {
        throw APIError(APIErrorType::BadParams, tr("Invalid public gateway settings."));
    }
    auto *manager = Net::PathManager::instance();
    if (!manager->setGatewayConfiguration({
        {u"controlAddress"_s, params().value(u"controlAddress"_s)},
        {u"datagramAddress"_s, params().value(u"datagramAddress"_s)},
        {u"serverName"_s, params().value(u"serverName"_s)},
        {u"caPath"_s, params().value(u"caPath"_s)},
        {u"certificatePath"_s, params().value(u"certificatePath"_s)},
        {u"privateKeyPath"_s, params().value(u"privateKeyPath"_s)},
        {u"port"_s, port}, {u"tcp"_s, tcpText == u"true"}, {u"udp"_s, udpText == u"true"}}))
    {
        throw APIError(APIErrorType::BadParams, manager->status());
    }
    statusAction();
}

void QbuttPathsController::resolveAction()
{
    requireParams({u"pathId"_s, u"generation"_s, u"host"_s, u"family"_s});
    requireIdle();
    bool valid = false;
    const quint64 generation = params().value(u"generation"_s).toULongLong(&valid);
    if (!valid || (generation == 0) || (generation > 9007199254740991))
        throw APIError(APIErrorType::BadParams, tr("Invalid path generation."));
    auto *manager = Net::PathManager::instance();
    if (manager->resolveHost(params().value(u"pathId"_s), generation,
        params().value(u"host"_s), params().value(u"family"_s)) == 0)
    {
        throw APIError(APIErrorType::BadParams, manager->status());
    }
    statusAction();
}

void QbuttPathsController::nativeAction()
{
    requireIdle();
    Net::PathManager::instance()->useNative();
    statusAction();
}

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
    if ((params().value(u"mode"_s) != u"mixed") && (params().value(u"mode"_s) != u"pinned"))
        throw APIError(APIErrorType::BadParams, tr("Unsupported network policy."));
    Net::PathManager::instance()->setPolicy(params().value(u"mode"_s), params().value(u"nativeInterface"_s));
    statusAction();
}

void QbuttPathsController::stopAction()
{
    requireIdle();
    Net::PathManager::instance()->stopPath(params().value(u"pathId"_s));
    statusAction();
}

void QbuttPathsController::nativeAction()
{
    requireIdle();
    Net::PathManager::instance()->useNative();
    statusAction();
}

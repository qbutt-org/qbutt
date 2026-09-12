/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "policiescontroller.h"

#include <QJsonDocument>

#include "base/bittorrent/completionpolicy.h"
#include "base/bittorrent/session.h"
#include "base/global.h"
#include "apierror.h"

void PoliciesController::configurationAction()
{
    setResult(BitTorrent::Session::instance()->completionPolicy()->configuration());
}

void PoliciesController::configureAction()
{
    requireParams({u"configuration"_s});
    const QJsonDocument document = QJsonDocument::fromJson(params()[u"configuration"_s].toUtf8());
    if (!document.isObject())
        throw APIError(APIErrorType::BadParams, tr("Configuration must be a JSON object."));
    if (const QString error = BitTorrent::Session::instance()->completionPolicy()->configure(document.object()); !error.isEmpty())
        throw APIError(APIErrorType::BadParams, error);
}

void PoliciesController::previewAction()
{
    setResult(BitTorrent::Session::instance()->completionPolicy()->preview());
}

void PoliciesController::journalAction()
{
    setResult(BitTorrent::Session::instance()->completionPolicy()->journal());
}

void PoliciesController::acknowledgeAction()
{
    requireParams({u"hash"_s, u"consent"_s});
    if (params()[u"consent"_s] != u"true")
        throw APIError(APIErrorType::BadParams, tr("Explicit consent=true is required to enable policies for an imported torrent."));
    const auto id = BitTorrent::TorrentID::fromString(params()[u"hash"_s]);
    if (const QString error = BitTorrent::Session::instance()->completionPolicy()->acknowledgePreview(id); !error.isEmpty())
        throw APIError(APIErrorType::Conflict, error);
}

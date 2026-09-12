/*
 * qbutt, a native Qt BitTorrent client.
 * Copyright (C) 2026 qbutt contributors
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the
 * Free Software Foundation; either version 2 of the License, or (at your
 * option) any later version.
 */

#include "repaircontroller.h"

#include <chrono>
#include <utility>

#include <QJsonArray>
#include <QUuid>

#include "base/bittorrent/infohash.h"
#include "base/bittorrent/repairanalysis.h"
#include "base/bittorrent/repairservice.h"
#include "base/bittorrent/session.h"
#include "base/global.h"
#include "apierror.h"

RepairController::RepairController(IApplication *app, QObject *parent)
    : APIController {app, parent}
    , m_status {{u"state"_s, u"idle"_s}}
{
    m_expiryTimer.setInterval(std::chrono::minutes {10});
    m_expiryTimer.setSingleShot(true);
    connect(&m_expiryTimer, &QTimer::timeout, this, [this]()
    {
        clearOperation();
        m_status = {{u"state"_s, u"expired"_s}, {u"error"_s, tr("Repair operation expired after 10 minutes without a request.")}};
    });
}

void RepairController::analyzeAction()
{
    requireParams({u"hash"_s});
    if (m_service)
        throw APIError(APIErrorType::Conflict, tr("Cancel the current repair operation before starting another."));

    const auto id = BitTorrent::TorrentID::fromString(params()[u"hash"_s]);
    BitTorrent::Torrent *const torrent = BitTorrent::Session::instance()->getTorrent(id);
    if (!torrent)
        throw APIError(APIErrorType::NotFound, tr("Torrent not found."));

    m_status =
    {
        {u"id"_s, QUuid::createUuid().toString(QUuid::WithoutBraces)},
        {u"hash"_s, id.toString()},
        {u"state"_s, u"analyzing"_s}
    };
    m_service = new BitTorrent::RepairService {torrent, this};
    connect(m_service, &BitTorrent::RepairService::analyzed, this, [this](const BitTorrent::RepairAnalysis &analysis)
    {
        QJsonArray files;
        for (const BitTorrent::RepairFileAnalysis &file : analysis.files)
        {
            files.append(QJsonObject
            {
                {u"native_index"_s, file.nativeIndex},
                {u"path"_s, file.path},
                {u"expected_size"_s, file.expectedSize},
                {u"actual_size"_s, file.actualSize},
                {u"verified_bytes"_s, file.verifiedBytes},
                {u"problems"_s, QJsonArray::fromStringList(file.problems)}
            });
        }
        m_status[u"analysis"_s] = QJsonObject
        {
            {u"files"_s, files},
            {u"expected_bytes"_s, analysis.expectedBytes},
            {u"verified_bytes"_s, analysis.verifiedBytes},
            {u"valid_pieces"_s, analysis.validPieces},
            {u"unverified_pieces"_s, analysis.unverifiedPieces},
            {u"whole_file_v2_verification"_s, analysis.wholeFileV2Verification}
        };
        m_status[u"state"_s] = u"analyzed"_s;
    });
    connect(m_service, &BitTorrent::RepairService::failed, this, [this](const QString &message)
    {
        m_status[u"state"_s] = u"failed"_s;
        m_status[u"error"_s] = message;
    });
    connect(m_service, &BitTorrent::RepairService::recheckStarted, this, [this]()
    {
        m_status[u"state"_s] = u"recheck_started"_s;
    });
    connect(m_service, &BitTorrent::RepairService::recheckFinished, this, [this]()
    {
        m_status[u"state"_s] = u"checked"_s;
    });

    m_expiryTimer.start();
    m_service->analyze();
    setResult(m_status);
}

void RepairController::statusAction()
{
    if (m_service)
        m_expiryTimer.start();
    setResult(m_status);
}

void RepairController::applyAction()
{
    requireParams({u"consent"_s});
    requireOperation();
    if (params()[u"consent"_s] != u"true")
        throw APIError(APIErrorType::BadParams, tr("Explicit consent=true is required for in-place repair without rollback."));
    if (m_status.value(u"state"_s).toString() != u"analyzed")
        throw APIError(APIErrorType::Conflict, tr("Repair requires a completed successful analysis."));

    m_status[u"state"_s] = u"applying"_s;
    m_expiryTimer.start();
    m_service->apply();
    setResult(m_status);
}

void RepairController::cancelAction()
{
    requireOperation();
    clearOperation();
    setResult(m_status);
}

void RepairController::requireOperation() const
{
    requireParams({u"id"_s});
    if (!m_service || (params()[u"id"_s] != m_status.value(u"id"_s).toString()))
        throw APIError(APIErrorType::Conflict, tr("The repair operation is no longer current."));
}

void RepairController::clearOperation()
{
    m_expiryTimer.stop();
    delete std::exchange(m_service, nullptr);
    m_status = {{u"state"_s, u"idle"_s}};
}

// Standalone integration driver linked to the application's native services.
// It only accepts generated profiles below its explicit lab directory.
#include <iostream>
#include <memory>
#include <stdexcept>

#include <libtorrent/torrent_info.hpp>

#include <QCoreApplication>
#include <QDir>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QSemaphore>
#include <QSettings>

#include "base/bittorrent/bencoderesumedatastorage.h"
#include "base/bittorrent/dbresumedatastorage.h"
#include "base/bittorrent/resumedatastorage.h"
#include "base/exceptions.h"
#include "base/logger.h"
#include "base/preferences.h"
#include "base/profile.h"
#include "base/profileimport.h"
#include "base/settingsstorage.h"

using namespace BitTorrent;

void require(const bool value, const char *message)
{
    if (!value)
        throw std::runtime_error(message);
}

void report(const QJsonObject &value)
{
    std::cout << QJsonDocument(value).toJson(QJsonDocument::Compact).constData() << std::endl;
}

int main(int argc, char **argv)
{
    QCoreApplication app(argc, argv);
    QCoreApplication::setApplicationName(QStringLiteral("qbutt"));
    try
    {
        require(argc >= 4, "mode root arguments required");
        const QString mode = QString::fromUtf8(argv[1]);
        const QString root = QDir::fromNativeSeparators(QString::fromUtf8(argv[2]));
        require(QDir::isAbsolutePath(root) && QFileInfo(root).fileName().startsWith(QStringLiteral("qbutt-profile-")), "isolated lab root required");
        const QString target = QDir(root).filePath(QString::fromUtf8(argv[3]));
        require(!QString::fromUtf8(argv[3]).contains(QStringLiteral("..")) && !QDir::isAbsolutePath(QString::fromUtf8(argv[3])), "relative fixture target required");
        Logger::initInstance();
        Profile::initInstance(Path(target), QString(), false);
        if (mode == QStringLiteral("seed-profile"))
        {
            require((argc == 7) || (argc == 8), "seed-profile root target backend fixtures subset [--preview-policy|--relative-path] required");
            SettingsStorage::initInstance();
            Preferences::initInstance();
            const QString backend = QString::fromUtf8(argv[4]);
            const QString fixtures = QString::fromUtf8(argv[5]);
            const QString subset = QString::fromUtf8(argv[6]);
            const Path data = specialFolderLocation(SpecialFolder::Data);
            QSemaphore written;
            bool saved = true;
            std::unique_ptr<ResumeDataStorage> storage;
            if (backend == QStringLiteral("db"))
                storage = std::make_unique<DBResumeDataStorage>(data / Path(QStringLiteral("torrents.db")));
            else
                storage = std::make_unique<BencodeResumeDataStorage>(data / Path(QStringLiteral("BT_backup")));
            QObject::connect(storage.get(), &ResumeDataStorage::stored, storage.get(), [&](quint64, bool success)
            {
                saved &= success;
                written.release();
            }, Qt::DirectConnection);
            quint64 revision = 0;
            for (const QString &kind : subset.split(QLatin1Char(',')))
            {
                LoadTorrentParams record;
                record.name = (kind == QStringLiteral("v2")) ? QString() : QStringLiteral("Generated ") + kind;
                record.category = QStringLiteral("fixture");
                record.stopped = true;
                record.completionPolicyPreview = (argc == 8) && (QString::fromUtf8(argv[7]) == QStringLiteral("--preview-policy"));
                record.ratioLimit = 0;
                record.shareLimitAction = ShareLimitAction::RemoveWithContent;
                record.savePath = Path(QDir(root).filePath(QString::fromUtf8(argv[3]) + QStringLiteral("-payload/") + kind));
                if ((argc == 8) && (QString::fromUtf8(argv[7]) == QStringLiteral("--relative-path")))
                    record.savePath = Path(QDir(root).relativeFilePath(record.savePath.data()));
                record.ltAddTorrentParams.ti = std::make_shared<lt::torrent_info>(QDir(fixtures).filePath(kind + QStringLiteral(".torrent")).toStdString());
                record.ltAddTorrentParams.save_path = record.savePath.data().toStdString();
                record.ltAddTorrentParams.flags |= lt::torrent_flags::upload_mode | lt::torrent_flags::share_mode;
                record.ltAddTorrentParams.flags &= ~lt::torrent_flags::apply_ip_filter;
                const auto id = TorrentID::fromInfoHash(InfoHash(record.ltAddTorrentParams.ti->info_hashes()));
                storage->store(id, record, ++revision);
            }
            require(written.tryAcquire(revision, 30000) && saved, "native source store failed");
            storage.reset();
            auto settings = Profile::instance()->applicationSettings(QStringLiteral("qbutt"));
            settings->setValue(QStringLiteral("Preferences/General/Locale"), QStringLiteral("en"));
            settings->setValue(QStringLiteral("BitTorrent/Session/MaxConnections"), 713);
            settings->setValue(QStringLiteral("BitTorrent/Session/GlobalDLSpeedLimit"), 456);
            settings->setValue(QStringLiteral("BitTorrent/Session/ResumeDataStorageType"), backend == QStringLiteral("db") ? QStringLiteral("SQLite") : QStringLiteral("Legacy"));
            settings->setValue(QStringLiteral("AutoRun/OnTorrentAddedEnabled"), true);
            settings->setValue(QStringLiteral("AutoRun/program"), QStringLiteral("must-not-be-imported"));
            settings->setValue(QStringLiteral("Network/Proxy/Password"), QStringLiteral("isolated-fixture-secret"));
            settings->sync();
            require(settings->status() == QSettings::NoError, "generated settings failed");
            report({{QStringLiteral("mode"), mode}, {QStringLiteral("data"), data.data()}, {QStringLiteral("settings"), settings->fileName()}, {QStringLiteral("count"), qint64(revision)}});
        }
        else if (mode == QStringLiteral("prepare"))
        {
            require(argc >= 7, "prepare root target sourceSettings sourceData sourceBase [fault] required");
            auto preview = ProfileImport::preview(Path(QString::fromUtf8(argv[4])), Path(QString::fromUtf8(argv[5])), Path(QString::fromUtf8(argv[6])));
            require(preview.has_value(), preview ? "" : qPrintable(preview.error()));
            require(preview->settings.contains(QStringLiteral("BitTorrent/Session/MaxConnections")), "safe setting missing");
            require(!preview->settings.contains(QStringLiteral("Network/Proxy/Password")), "private setting imported");
            const QString fault = (argc > 7) ? QString::fromUtf8(argv[7]) : QString();
            if (fault == QStringLiteral("bad-index"))
                preview->torrents[0].params.ltAddTorrentParams.renamed_files[lt::file_index_t(99999)] = "unexpected.bin";
            if (fault == QStringLiteral("protected"))
                preview->torrents[0].destinationPath = specialFolderLocation(SpecialFolder::Data);
            if (fault == QStringLiteral("overlap"))
                preview->torrents[1].destinationPath = preview->torrents[0].destinationPath;
            if (fault == QStringLiteral("mapped-alias"))
            {
                for (auto &torrent : preview->torrents)
                    torrent.selected = false;
                auto &torrent = preview->torrents[0];
                torrent.selected = true;
                torrent.destinationPath = Path(QDir(root).filePath(QString::fromUtf8(argv[3]) + QStringLiteral("-payload")));
                torrent.params.ltAddTorrentParams.renamed_files[lt::file_index_t(0)] = "bundle/alpha.bin.!qB";
            }
            const auto result = ProfileImport::prepare(std::move(*preview), fault != QStringLiteral("no-consent"));
            report({{QStringLiteral("mode"), mode}, {QStringLiteral("success"), bool(result)}, {QStringLiteral("error"), result ? QString() : result.error()}});
            return result ? 0 : 2;
        }
        else if (mode == QStringLiteral("recover"))
        {
            const auto result = ProfileImport::recover();
            report({{QStringLiteral("mode"), mode}, {QStringLiteral("success"), bool(result)}, {QStringLiteral("error"), result ? QString() : result.error()}});
            return result ? 0 : 2;
        }
        else if (mode == QStringLiteral("inspect"))
        {
            const auto records = ResumeDataStorage::readExternal(specialFolderLocation(SpecialFolder::Data), Path(QDir(target).filePath(QStringLiteral("qbutt"))));
            require(bool(records), records ? "" : qPrintable(records.error()));
            QJsonArray torrents;
            for (const auto &record : *records)
            {
                require(bool(record.result), "native record unreadable");
                const auto &p = *record.result;
                torrents.append(QJsonObject {{QStringLiteral("id"), record.torrentID.toString()}, {QStringLiteral("stopped"), p.stopped}
                    , {QStringLiteral("preview"), p.completionPolicyPreview}, {QStringLiteral("savePath"), p.savePath.data()}
                    , {QStringLiteral("finished"), p.hasFinishedStatus}, {QStringLiteral("renamedCount"), qint64(p.ltAddTorrentParams.renamed_files.size())}
                    , {QStringLiteral("nativePath"), Path(p.ltAddTorrentParams.save_path).data()}
                    , {QStringLiteral("autoTMM"), p.useAutoTMM}, {QStringLiteral("haveCount"), p.ltAddTorrentParams.have_pieces.count()}
                    , {QStringLiteral("uploadMode"), bool(p.ltAddTorrentParams.flags & lt::torrent_flags::upload_mode)}
                    , {QStringLiteral("shareMode"), bool(p.ltAddTorrentParams.flags & lt::torrent_flags::share_mode)}
                    , {QStringLiteral("applyIPFilter"), bool(p.ltAddTorrentParams.flags & lt::torrent_flags::apply_ip_filter)}});
            }
            report({{QStringLiteral("mode"), mode}, {QStringLiteral("torrents"), torrents}});
        }
        else
            throw std::runtime_error("unknown integration mode");
        return 0;
    }
    catch (const RuntimeError &error)
    {
        report({{QStringLiteral("error"), error.message()}});
    }
    catch (const std::exception &error)
    {
        report({{QStringLiteral("error"), QString::fromUtf8(error.what())}});
    }
    return 1;
}

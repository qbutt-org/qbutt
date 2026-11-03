/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include <atomic>
#include <cerrno>
#include <csignal>
#include <cstdio>
#include <iterator>
#include <memory>
#include <stdexcept>
#include <thread>
#include <vector>

#include <fcntl.h>
#include <poll.h>
#include <sys/inotify.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#include <libtorrent/bencode.hpp>
#include <libtorrent/create_torrent.hpp>
#include <libtorrent/file_storage.hpp>
#include <libtorrent/torrent_info.hpp>

#include <QCoreApplication>
#include <QCryptographicHash>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QTemporaryDir>

#include "base/bittorrent/repairanalysis.h"
#include "base/bittorrent/repairfileguard.h"

namespace
{
    void require(const bool condition, const QString &message)
    {
        if (!condition)
            throw std::runtime_error(message.toStdString());
    }

    void writeFile(const QString &path, const QByteArray &data)
    {
        require(QDir().mkpath(QFileInfo(path).absolutePath()), QStringLiteral("Cannot create fixture directory"));
        QFile output(path);
        require(output.open(QIODevice::WriteOnly) && (output.write(data) == data.size()), QStringLiteral("Cannot write fixture"));
    }

    QByteArray readFile(const QString &path)
    {
        QFile input(path);
        require(input.open(QIODevice::ReadOnly), QStringLiteral("Cannot read fixture"));
        return input.readAll();
    }

    QByteArray fileHash(const QString &path)
    {
        QFile input(path);
        require(input.open(QIODevice::ReadOnly), QStringLiteral("Cannot hash fixture"));
        QCryptographicHash hash(QCryptographicHash::Sha256);
        require(hash.addData(&input), QStringLiteral("Cannot hash fixture bytes"));
        return hash.result();
    }

    std::unique_ptr<lt::torrent_info> makeTorrent(const QString &root, lt::file_storage files
        , const lt::create_flags_t flags = lt::create_torrent::v1_only)
    {
        lt::create_torrent creator(files, 16 * 1024, flags);
        lt::error_code error;
        lt::set_piece_hashes(creator, root.toStdString(), error);
        require(!error, QString::fromStdString(error.message()));
        std::vector<char> metadata;
        lt::bencode(std::back_inserter(metadata), creator.generate());
        auto target = std::make_unique<lt::torrent_info>(metadata.data(), int(metadata.size()), error);
        require(!error, QString::fromStdString(error.message()));
        return target;
    }

    std::shared_ptr<BitTorrent::RepairFileGuard> openGuard(const lt::torrent_info &target, const QString &root)
    {
        QString error;
        auto guard = BitTorrent::RepairFileGuard::open(target.files(), root, false, error);
        require(bool(guard), error);
        return guard;
    }

    void rejected(const lt::torrent_info &target, const QString &root)
    {
        QString error;
        require(!BitTorrent::RepairFileGuard::open(target.files(), root, false, error) && !error.isEmpty()
            , QStringLiteral("Unsafe fixture was accepted"));
    }

    // A real process attempts an ordinary non-cooperating write open. No flock
    // or other cooperative protocol is involved.
    int externalWriter(const QString &path)
    {
        const QByteArray encoded = QFile::encodeName(path);
        const pid_t child = fork();
        require(child >= 0, QStringLiteral("Cannot create writer process"));
        if (child == 0)
        {
            const int descriptor = open(encoded.constData(), O_WRONLY | O_NONBLOCK | O_CLOEXEC);
            if (descriptor >= 0)
            {
                close(descriptor);
                _exit(0);
            }
            _exit((errno == EWOULDBLOCK) ? 10 : 11);
        }
        int status = 0;
        require((waitpid(child, &status, 0) == child) && WIFEXITED(status), QStringLiteral("Writer process failed"));
        return WEXITSTATUS(status);
    }
}

int main(int argc, char **argv)
{
    QCoreApplication application(argc, argv);
    QJsonArray checks;
    try
    {
        QTemporaryDir directory;
        require(directory.isValid(), QStringLiteral("Cannot create isolated fixture root"));
        const QString root = directory.path();
        const QString alphaPath = root + QStringLiteral("/bundle/alpha.bin");
        const QString betaPath = root + QStringLiteral("/bundle/nested/beta.bin");
        const QByteArray alpha(16507, 'a');
        const QByteArray beta(49159, 'b');
        writeFile(alphaPath, alpha);
        writeFile(betaPath, beta);
        writeFile(root + QStringLiteral("/bundle/empty.bin"), {});
        writeFile(root + QStringLiteral("/bundle/user-notes.txt"), QByteArrayLiteral("preserve unrelated data"));
        lt::file_storage files;
        files.add_file("bundle/alpha.bin", alpha.size());
        files.add_file("bundle/nested/beta.bin", beta.size());
        files.add_file("bundle/empty.bin", 0);

        for (const lt::create_flags_t flags : {lt::create_torrent::v1_only, lt::create_torrent::v2_only, lt::create_flags_t {}})
        {
            const auto target = makeTorrent(root, files, flags);
            auto guard = openGuard(*target, root);
            const BitTorrent::RepairAnalysis analysis = guard->analyze(*target, target->files());
            require(analysis.error.isEmpty() && (analysis.verifiedBytes == alpha.size() + beta.size())
                && (analysis.unverifiedPieces == 0), analysis.error + QStringLiteral(" Incorrect v1/v2/hybrid verification"));
            checks.append(QJsonObject {{QStringLiteral("scenario"), target->v1() ? (target->v2() ? "hybrid" : "v1") : "v2"}
                , {QStringLiteral("verifiedBytes"), analysis.verifiedBytes}});
        }

        const auto target = makeTorrent(root, files);
        {
            const QByteArray bundle = QFile::encodeName(root + QStringLiteral("/bundle"));
            require(chmod(bundle.constData(), 0100) == 0, QStringLiteral("Cannot prepare search-only directory"));
            QString directoryError;
            auto guard = BitTorrent::RepairFileGuard::open(target->files(), root, false, directoryError);
            const BitTorrent::RepairAnalysis analysis = guard ? guard->analyze(*target, target->files()) : BitTorrent::RepairAnalysis {};
            require(chmod(bundle.constData(), 0700) == 0, QStringLiteral("Cannot restore fixture directory permissions"));
            require(guard && analysis.error.isEmpty() && (analysis.verifiedBytes == alpha.size() + beta.size())
                , directoryError + analysis.error + QStringLiteral(" Search-only parent could not be traversed"));
        }
        checks.append(QStringLiteral("search-only-parent-directory"));
        writeFile(alphaPath, alpha + QByteArrayLiteral("oversized tail"));
        {
            auto guard = openGuard(*target, root);
            const BitTorrent::RepairAnalysis analysis = guard->analyze(*target, target->files());
            require(analysis.error.isEmpty() && (analysis.verifiedBytes == alpha.size() + beta.size())
                && (analysis.files.front().actualSize > analysis.files.front().expectedSize), QStringLiteral("Oversized tail was lost or miscounted"));
            QString error;
            require(!guard->truncateOversized(error) && !error.isEmpty(), QStringLiteral("Read-only analysis permitted truncation"));
            require(!guard->createMissingEmpty(error) && !error.isEmpty(), QStringLiteral("Read-only analysis permitted file creation"));
        }
        require(readFile(alphaPath) == alpha + QByteArrayLiteral("oversized tail"), QStringLiteral("Analysis changed payload"));
        writeFile(alphaPath, alpha);
        checks.append(QStringLiteral("oversized-read-only"));

        {
            lt::file_storage missing = target->files();
            missing.rename_file(lt::file_index_t(2), "bundle/absent/deep/empty.bin");
            QString error;
            auto guard = BitTorrent::RepairFileGuard::open(missing, root, false, error);
            require(bool(guard), error);
            const BitTorrent::RepairAnalysis analysis = guard->analyze(*target, missing);
            require(analysis.error.isEmpty() && !QFileInfo::exists(root + QStringLiteral("/bundle/absent")), QStringLiteral("Analysis created missing directories"));
            require(QDir().mkpath(root + QStringLiteral("/bundle/absent")), QStringLiteral("Cannot introduce directory race"));
            require(!guard->analyze(*target, missing).error.isEmpty(), QStringLiteral("Missing-directory race was accepted"));
            require(QDir(root + QStringLiteral("/bundle/absent")).removeRecursively(), QStringLiteral("Cannot remove owned race directory"));
        }
        checks.append(QStringLiteral("missing-directory-race"));

        {
            lt::file_storage missing = target->files();
            missing.rename_file(lt::file_index_t(0), "bundle/absent.bin");
            QString error;
            auto guard = BitTorrent::RepairFileGuard::open(missing, root, false, error);
            require(bool(guard), error);
            const BitTorrent::RepairAnalysis analysis = guard->analyze(*target, missing);
            require(analysis.error.isEmpty() && (analysis.verifiedBytes < analysis.expectedBytes), QStringLiteral("Missing payload was reported complete"));
            writeFile(root + QStringLiteral("/bundle/absent.bin"), QByteArrayLiteral("unrelated new file"));
            require(!guard->analyze(*target, missing).error.isEmpty(), QStringLiteral("Missing-file race was accepted"));
            require(QFile::remove(root + QStringLiteral("/bundle/absent.bin")), QStringLiteral("Cannot remove owned missing-file race"));
        }
        checks.append(QStringLiteral("missing-file-race"));

        const QString alias = root + QStringLiteral("/alias.bin");
        require(link(QFile::encodeName(alphaPath).constData(), QFile::encodeName(alias).constData()) == 0, QStringLiteral("Cannot create hardlink fixture"));
        rejected(*target, root);
        require(readFile(alias) == alpha, QStringLiteral("Rejected hardlink changed data"));
        require(QFile::remove(alias), QStringLiteral("Cannot remove hardlink fixture"));
        {
            auto guard = openGuard(*target, root);
            require(link(QFile::encodeName(alphaPath).constData(), QFile::encodeName(alias).constData()) == 0, QStringLiteral("Cannot introduce hardlink race"));
            require(!guard->analyze(*target, target->files()).error.isEmpty(), QStringLiteral("Hardlink race was accepted"));
        }
        require(QFile::remove(alias), QStringLiteral("Cannot remove hardlink race"));
        checks.append(QStringLiteral("hardlink-and-race"));

        require(QFile::rename(alphaPath, alias), QStringLiteral("Cannot prepare symlink fixture"));
        require(symlink(QFile::encodeName(alias).constData(), QFile::encodeName(alphaPath).constData()) == 0, QStringLiteral("Cannot create symlink fixture"));
        rejected(*target, root);
        require(QFile::remove(alphaPath) && QFile::rename(alias, alphaPath), QStringLiteral("Cannot restore symlink fixture"));
        const QString dangling = root + QStringLiteral("/dangling");
        require(symlink("absent", QFile::encodeName(dangling).constData()) == 0, QStringLiteral("Cannot create dangling symlink"));
        require(BitTorrent::repairPathIdentity(dangling).isEmpty(), QStringLiteral("Dangling alias became a missing lexical suffix"));
        require(QFile::remove(dangling), QStringLiteral("Cannot remove dangling symlink"));
        require(QFile::rename(root + QStringLiteral("/bundle/nested"), root + QStringLiteral("/other")), QStringLiteral("Cannot prepare directory symlink"));
        require(symlink("../other", QFile::encodeName(root + QStringLiteral("/bundle/nested")).constData()) == 0, QStringLiteral("Cannot create directory symlink"));
        rejected(*target, root);
        require(QFile::remove(root + QStringLiteral("/bundle/nested"))
            && QFile::rename(root + QStringLiteral("/other"), root + QStringLiteral("/bundle/nested")), QStringLiteral("Cannot restore directory fixture"));
        checks.append(QStringLiteral("file-directory-and-dangling-symlinks"));

        require(QFile::rename(alphaPath, alias), QStringLiteral("Cannot prepare FIFO fixture"));
        require(mkfifo(QFile::encodeName(alphaPath).constData(), 0600) == 0, QStringLiteral("Cannot create FIFO fixture"));
        rejected(*target, root);
        require(QFile::remove(alphaPath) && QFile::rename(alias, alphaPath), QStringLiteral("Cannot restore FIFO fixture"));
        checks.append(QStringLiteral("fifo-does-not-block"));

        {
            auto guard = openGuard(*target, root);
            require(QFile::rename(alphaPath, alias), QStringLiteral("Cannot introduce path replacement"));
            writeFile(alphaPath, QByteArrayLiteral("unrelated replacement"));
            require(!guard->analyze(*target, target->files()).error.isEmpty(), QStringLiteral("Path replacement was accepted"));
            require(readFile(alias) == alpha && readFile(alphaPath) == QByteArrayLiteral("unrelated replacement"), QStringLiteral("Path replacement damaged data"));
        }
        require(QFile::remove(alphaPath) && QFile::rename(alias, alphaPath), QStringLiteral("Cannot restore replaced path"));
        checks.append(QStringLiteral("path-replacement"));

        {
            const QString racePath = root + QStringLiteral("/concurrent/data.bin");
            const QString original = root + QStringLiteral("/concurrent/original.bin");
            constexpr qint64 RaceBytes = 256 * 1024 * 1024;
            writeFile(racePath, {});
            QFile sparse(racePath);
            require(sparse.open(QIODevice::ReadWrite) && sparse.resize(RaceBytes), QStringLiteral("Cannot create sparse race payload"));
            sparse.close();
            const QByteArray originalHash = fileHash(racePath);
            lt::file_storage raceFiles;
            raceFiles.add_file("concurrent/data.bin", RaceBytes);
            const auto raceTarget = makeTorrent(root, raceFiles);
            const int events = inotify_init1(IN_CLOEXEC | IN_NONBLOCK);
            require(events >= 0, QStringLiteral("Cannot observe actual hashing reads"));
            require(inotify_add_watch(events, QFile::encodeName(racePath).constData(), IN_ACCESS) >= 0, QStringLiteral("Cannot watch race payload"));
            auto guard = openGuard(*raceTarget, root);
            std::atomic_int raceStatus {0};
            std::jthread racer([&]
            {
                struct pollfd event {events, POLLIN, 0};
                if (poll(&event, 1, 5000) != 1)
                    return;
                const QByteArray from = QFile::encodeName(racePath);
                const QByteArray to = QFile::encodeName(original);
                if (rename(from.constData(), to.constData()) != 0)
                    return;
                const int replacement = open(from.constData(), O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0600);
                if (replacement < 0)
                    return;
                const char contents[] = "unrelated replacement";
                const bool written = write(replacement, contents, sizeof(contents) - 1) == ssize_t(sizeof(contents) - 1);
                close(replacement);
                raceStatus.store(written ? 1 : -1);
            });
            const BitTorrent::RepairAnalysis raced = guard->analyze(*raceTarget, raceTarget->files());
            racer.join();
            close(events);
            require(raceStatus.load() == 1, QStringLiteral("Concurrent replacement did not run during actual file reads"));
            require(!raced.error.isEmpty() && (raced.verifiedBytes == 0), QStringLiteral("Concurrent path replacement published verification"));
            require(fileHash(original) == originalHash && readFile(racePath) == QByteArrayLiteral("unrelated replacement"), QStringLiteral("Concurrent replacement damaged original or unrelated data"));
        }
        checks.append(QStringLiteral("replacement-during-actual-hashing"));

        {
            const int writer = open(QFile::encodeName(alphaPath).constData(), O_RDWR | O_CLOEXEC);
            require(writer >= 0, QStringLiteral("Cannot open preexisting writer"));
            rejected(*target, root);
            void *mapping = mmap(nullptr, alpha.size(), PROT_READ | PROT_WRITE, MAP_SHARED, writer, 0);
            require(mapping != MAP_FAILED, QStringLiteral("Cannot create mapped writer"));
            close(writer);
            rejected(*target, root);
            require(munmap(mapping, alpha.size()) == 0, QStringLiteral("Cannot release mapped writer"));
        }
        {
            struct sigaction beforeSignal {};
            require(sigaction(SIGURG, nullptr, &beforeSignal) == 0, QStringLiteral("Cannot inspect signal disposition"));
            auto guard = openGuard(*target, root);
            require(externalWriter(alphaPath) == 10, QStringLiteral("Native read lease did not block an external writer"));
            const BitTorrent::RepairAnalysis invalid = guard->analyze(*target, target->files());
            require(!invalid.error.isEmpty() && (invalid.verifiedBytes == 0), QStringLiteral("A broken read lease published verified bytes"));
            struct sigaction afterSignal {};
            require((sigaction(SIGURG, nullptr, &afterSignal) == 0) && (beforeSignal.sa_handler == afterSignal.sa_handler)
                , QStringLiteral("Read lease changed process-global signal disposition"));
        }
        require(externalWriter(alphaPath) == 0, QStringLiteral("Lease release did not unblock ordinary writers"));
        checks.append(QStringLiteral("existing-mapped-and-external-writers"));

        QString error;
        require(!BitTorrent::RepairFileGuard::open(target->files(), root, true, error)
            && error.contains(QStringLiteral("exclusive")), QStringLiteral("Linux mutation capability was overstated"));
        require(!BitTorrent::RepairFileGuard::open(target->files(), root, false, error, nullptr, true), QStringLiteral("Linux rename ownership was overstated"));
        require(!BitTorrent::RepairFileGuard::open(lt::file_storage {}, QStringLiteral("/proc"), false, error)
            && error.contains(QStringLiteral("filesystem")), QStringLiteral("An unsupported filesystem was accepted"));
        std::atomic_bool cancelFlag {true};
        require(!BitTorrent::RepairFileGuard::open(target->files(), root, false, error, &cancelFlag), QStringLiteral("Cancellation was ignored"));
        checks.append(QStringLiteral("capability-filesystem-and-cancellation"));

        writeFile(root + QStringLiteral("/bundle/Case.bin"), QByteArrayLiteral("upper"));
        writeFile(root + QStringLiteral("/bundle/case.bin"), QByteArrayLiteral("lower"));
        lt::file_storage caseFiles;
        caseFiles.add_file("bundle/Case.bin", 5);
        caseFiles.add_file("bundle/case.bin", 5);
        const auto caseTarget = makeTorrent(root, caseFiles);
        {
            // libtorrent may sanitize case collisions in metadata. The engine's
            // explicit physical mapping still names two distinct Linux files.
            auto guard = BitTorrent::RepairFileGuard::open(caseFiles, root, false, error);
            require(bool(guard), error);
            const BitTorrent::RepairAnalysis analysis = guard->analyze(*caseTarget, caseFiles);
            require(analysis.error.isEmpty() && (analysis.verifiedBytes == 10)
                , analysis.error + QStringLiteral(" Case-distinct mapped files were not verified"));
        }
        require(BitTorrent::repairPathIdentity(root + QStringLiteral("/bundle/Case.bin")) != BitTorrent::repairPathIdentity(root + QStringLiteral("/bundle/case.bin"))
            , QStringLiteral("Linux path identities were case-folded"));
        require(BitTorrent::repairPathIdentity(QStringLiteral("/")) == u"/", QStringLiteral("Linux root identity is empty"));
        checks.append(QStringLiteral("case-sensitive-identities"));

        const qsizetype before = QDir(QStringLiteral("/proc/self/fd")).entryList(QDir::AllEntries | QDir::NoDotAndDotDot).size();
        for (int i = 0; i < 40; ++i)
        {
            auto guard = openGuard(*target, root);
            require(guard->analyze(*target, target->files()).error.isEmpty(), QStringLiteral("Repeated snapshot failed"));
        }
        const qsizetype after = QDir(QStringLiteral("/proc/self/fd")).entryList(QDir::AllEntries | QDir::NoDotAndDotDot).size();
        require(before == after, QStringLiteral("Snapshot descriptors leaked"));
        require(readFile(alphaPath) == alpha && readFile(betaPath) == beta
            && readFile(root + QStringLiteral("/bundle/user-notes.txt")) == QByteArrayLiteral("preserve unrelated data"), QStringLiteral("Read-only scenarios damaged payload"));
        checks.append(QStringLiteral("descriptor-lifetime-and-payload-preservation"));

        const QJsonObject receipt {{QStringLiteral("status"), QStringLiteral("passed")}, {QStringLiteral("checks"), checks}};
        std::puts(QJsonDocument(receipt).toJson(QJsonDocument::Compact).constData());
        return 0;
    }
    catch (const std::exception &error)
    {
        std::fprintf(stderr, "%s\n", error.what());
        return 1;
    }
}

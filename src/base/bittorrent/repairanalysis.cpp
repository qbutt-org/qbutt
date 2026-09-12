/*
 * qbutt, a native Qt BitTorrent client.
 * Copyright (C) 2026 qbutt contributors
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the
 * Free Software Foundation; either version 2 of the License, or (at your
 * option) any later version.
 */

#include "repairanalysis.h"

#include <algorithm>
#include <array>
#include <bit>
#include <cstdint>
#include <utility>
#include <vector>

#include <libtorrent/bitfield.hpp>
#include <libtorrent/file_storage.hpp>
#include <libtorrent/hasher.hpp>
#include <libtorrent/torrent_info.hpp>

#include <QCoreApplication>
#include <QFile>
#include <QFileInfo>

namespace
{
    constexpr int BlockSize = 16 * 1024;

    lt::sha256_hash merkleParent(const lt::sha256_hash &left, const lt::sha256_hash &right)
    {
        lt::hasher256 parent;
        parent.update(left.data(), lt::sha256_hash::size());
        parent.update(right.data(), lt::sha256_hash::size());
        return parent.final();
    }

    // Streaming BEP 52 tree reduction keeps one pending hash per level rather
    // than allocating a hash for every block in a potentially very large file.
    class MerkleAccumulator
    {
    public:
        void append(lt::sha256_hash hash);
        lt::sha256_hash root(std::uint64_t minimumLeaves = 1, lt::sha256_hash padding = {}) const;

    private:
        std::array<lt::sha256_hash, 64> m_levels {};
        std::uint64_t m_leafCount = 0;
    };

    void MerkleAccumulator::append(lt::sha256_hash hash)
    {
        unsigned int level = 0;
        for (std::uint64_t occupied = m_leafCount; (occupied & 1) != 0; occupied >>= 1)
        {
            hash = merkleParent(m_levels[level], hash);
            ++level;
        }
        m_levels[level] = hash;
        ++m_leafCount;
    }

    lt::sha256_hash MerkleAccumulator::root(const std::uint64_t minimumLeaves, lt::sha256_hash padding) const
    {
        const std::uint64_t leaves = std::bit_ceil(std::max(m_leafCount, minimumLeaves));
        const int height = std::countr_zero(leaves);
        if (m_leafCount == leaves)
            return m_levels[height];

        lt::sha256_hash hash = padding;
        for (int level = 0; level < height; ++level)
        {
            hash = ((m_leafCount >> level) & 1) != 0
                ? merkleParent(m_levels[level], hash) : merkleParent(hash, padding);
            padding = merkleParent(padding, padding);
        }
        return hash;
    }

    bool checkCancellation(BitTorrent::RepairAnalysis &result, const std::atomic_bool *cancelled)
    {
        if (!cancelled || !cancelled->load(std::memory_order_relaxed))
            return false;
        result.error = QCoreApplication::translate("RepairAnalysis", "Analysis cancelled.");
        return true;
    }

    void recordProblem(BitTorrent::RepairFileAnalysis &file, const QString &problem)
    {
        if (!file.problems.contains(problem))
            file.problems.append(problem);
    }

    bool openInput(QFile &input, const BitTorrent::RepairFileAnalysis &file, const QMap<int, int> *descriptors)
    {
        if (descriptors)
            return input.open(descriptors->value(file.nativeIndex, -1), QIODevice::ReadOnly, QFileDevice::DontCloseHandle)
                && input.seek(0);
        input.setFileName(file.path);
        return input.open(QIODevice::ReadOnly);
    }

    void analyzeV1(const lt::torrent_info &target, BitTorrent::RepairAnalysis &result
        , const std::vector<int> &fileReports, lt::bitfield &verified, const std::atomic_bool *cancelled, const QMap<int, int> *descriptors)
    {
        std::array<char, BlockSize> buffer {};
        const lt::file_storage &files = target.files();
        for (const lt::piece_index_t piece : files.piece_range())
        {
            if (checkCancellation(result, cancelled))
                return;
            const std::vector<lt::file_slice> slices = target.map_block(piece, 0, target.piece_size(piece));
            lt::hasher hasher;
            bool readable = true;
            for (const lt::file_slice &slice : slices)
            {
                QFile input;
                if (files.pad_file_at(slice.file_index))
                {
                    buffer.fill(0);
                }
                else
                {
                    BitTorrent::RepairFileAnalysis &file = result.files[fileReports[int(slice.file_index)]];
                    if (file.actualSize < 0)
                    {
                        readable = false;
                        break;
                    }
                    if (!openInput(input, file, descriptors) || !input.seek(slice.offset))
                    {
                        recordProblem(file, QCoreApplication::translate("RepairAnalysis", "Cannot read the required file data."));
                        readable = false;
                        break;
                    }
                }

                for (qint64 remaining = slice.size; remaining > 0;)
                {
                    if (checkCancellation(result, cancelled))
                        return;
                    const int size = static_cast<int>(std::min<qint64>(remaining, buffer.size()));
                    if (!files.pad_file_at(slice.file_index) && (input.read(buffer.data(), size) != size))
                    {
                        recordProblem(result.files[fileReports[int(slice.file_index)]]
                            , QCoreApplication::translate("RepairAnalysis", "Cannot read the required file data."));
                        readable = false;
                        break;
                    }
                    hasher.update(buffer.data(), size);
                    remaining -= size;
                }
                if (!readable)
                    break;
            }

            if (readable && (hasher.final() == target.hash_for_piece(piece)))
            {
                verified.set_bit(int(piece));
            }
            else if (readable)
            {
                for (const lt::file_slice &slice : slices)
                {
                    if (!files.pad_file_at(slice.file_index))
                    {
                        recordProblem(result.files[fileReports[int(slice.file_index)]]
                            , QCoreApplication::translate("RepairAnalysis", "A v1 piece covering this file does not match its expected hash."));
                    }
                }
            }
        }
    }

    void analyzeV2(const lt::torrent_info &target, BitTorrent::RepairAnalysis &result
        , const std::vector<int> &fileReports, lt::bitfield &verified, const std::atomic_bool *cancelled, const QMap<int, int> *descriptors)
    {
        const lt::file_storage &files = target.files();
        std::array<char, BlockSize> buffer {};
        for (const lt::file_index_t index : files.file_range())
        {
            if (checkCancellation(result, cancelled))
                return;
            if (files.pad_file_at(index) || (files.file_size(index) == 0))
                continue;

            BitTorrent::RepairFileAnalysis &file = result.files[fileReports[int(index)]];
            if (file.actualSize < 0)
                continue;
            const int numPieces = files.file_num_pieces(index);
            const int firstPiece = int(files.piece_index_at_file(index));
            const lt::span<const char> pieceLayer = target.piece_layer(index);
            const bool wholeFile = (numPieces > 1) && pieceLayer.empty();
            result.wholeFileV2Verification |= wholeFile;

            if (!wholeFile && (numPieces > 1))
            {
                // Piece layers are outside the info dictionary. Authenticate
                // them against the file root before treating them as evidence.
                if (pieceLayer.size() != (std::int64_t(numPieces) * lt::sha256_hash::size()))
                {
                    recordProblem(file, QCoreApplication::translate("RepairAnalysis", "The v2 piece layer has an invalid length."));
                    continue;
                }
                MerkleAccumulator layer;
                for (int piece = 0; piece < numPieces; ++piece)
                {
                    if (checkCancellation(result, cancelled))
                        return;
                    layer.append(lt::sha256_hash(pieceLayer.data() + (piece * lt::sha256_hash::size())));
                }
                MerkleAccumulator padding;
                if (layer.root(1, padding.root(files.blocks_per_piece())) != files.root(index))
                {
                    recordProblem(file, QCoreApplication::translate("RepairAnalysis", "The v2 piece layer does not match the file root hash."));
                    continue;
                }
            }

            QFile input;
            if (!openInput(input, file, descriptors))
            {
                recordProblem(file, QCoreApplication::translate("RepairAnalysis", "Cannot read the required file data."));
                continue;
            }

            const int checks = wholeFile ? 1 : numPieces;
            bool readable = true;
            for (int piece = 0; (piece < checks) && readable; ++piece)
            {
                const qint64 offset = qint64(piece) * files.piece_length();
                const qint64 size = wholeFile ? file.expectedSize
                    : std::min<qint64>(files.piece_length(), file.expectedSize - offset);
                MerkleAccumulator hashes;
                for (qint64 remaining = size; remaining > 0;)
                {
                    if (checkCancellation(result, cancelled))
                        return;
                    const int blockSize = static_cast<int>(std::min<qint64>(remaining, buffer.size()));
                    if (input.read(buffer.data(), blockSize) != blockSize)
                    {
                        recordProblem(file, QCoreApplication::translate("RepairAnalysis", "Cannot read the required file data."));
                        readable = false;
                        break;
                    }
                    hashes.append(lt::hasher256(buffer.data(), blockSize).final());
                    remaining -= blockSize;
                }
                if (!readable)
                    break;

                const bool fileRoot = wholeFile || (numPieces == 1);
                const lt::sha256_hash expected = fileRoot ? files.root(index)
                    : lt::sha256_hash(pieceLayer.data() + (piece * lt::sha256_hash::size()));
                const lt::sha256_hash actual = hashes.root(fileRoot ? 1 : files.blocks_per_piece());
                if (actual == expected)
                {
                    const int count = wholeFile ? numPieces : 1;
                    for (int verifiedPiece = 0; verifiedPiece < count; ++verifiedPiece)
                    {
                        if (checkCancellation(result, cancelled))
                            return;
                        verified.set_bit(firstPiece + piece + verifiedPiece);
                    }
                }
                else
                {
                    recordProblem(file, wholeFile
                        ? QCoreApplication::translate("RepairAnalysis", "The v2 file root does not match; individual pieces cannot be verified without a piece layer.")
                        : QCoreApplication::translate("RepairAnalysis", "A v2 piece does not match its expected hash."));
                }
            }
        }
    }
}

BitTorrent::RepairAnalysis BitTorrent::analyzeRepairData(const lt::torrent_info &target
    , const lt::file_storage &mappedFiles, const QString &savePath, const std::atomic_bool *cancelled
    , const QSet<int> *readableFiles, const QMap<int, int> *readDescriptors)
{
    RepairAnalysis result;
    const lt::file_storage &files = target.files();
    if (!target.is_valid() || (files.num_files() != mappedFiles.num_files()))
    {
        result.error = QCoreApplication::translate("RepairAnalysis", "The torrent metadata or file mapping is invalid.");
        return result;
    }

    std::vector<int> fileReports(files.num_files(), -1);
    for (const lt::file_index_t index : files.file_range())
    {
        if (checkCancellation(result, cancelled))
            return result;
        if (files.pad_file_at(index))
            continue;

        RepairFileAnalysis file;
        file.nativeIndex = int(index);
        file.path = QString::fromStdString(mappedFiles.file_path(index, savePath.toStdString()));
        file.expectedSize = files.file_size(index);
        if (!readableFiles || readableFiles->contains(int(index)))
        {
            const QFileInfo fileInfo {file.path};
            QFile input;
            if (readDescriptors ? openInput(input, file, readDescriptors) : fileInfo.exists())
            {
                file.actualSize = readDescriptors ? input.size() : fileInfo.size();
                if (file.actualSize != file.expectedSize)
                {
                    file.problems.append(QCoreApplication::translate("RepairAnalysis", "File size is %1 bytes; expected exactly %2 bytes.")
                        .arg(file.actualSize).arg(file.expectedSize));
                }
            }
        }
        if (file.actualSize < 0)
        {
            file.problems.append(QCoreApplication::translate("RepairAnalysis", "File is missing."));
        }
        fileReports[int(index)] = static_cast<int>(result.files.size());
        result.expectedBytes += file.expectedSize;
        result.files.append(std::move(file));
    }

    lt::bitfield v1Verified {target.num_pieces(), !target.v1()};
    lt::bitfield v2Verified {target.num_pieces(), !target.v2()};
    if (target.v1())
        analyzeV1(target, result, fileReports, v1Verified, cancelled, readDescriptors);
    if (target.v2() && result.error.isEmpty())
        analyzeV2(target, result, fileReports, v2Verified, cancelled, readDescriptors);
    if (!result.error.isEmpty())
        return result;

    for (const lt::piece_index_t piece : files.piece_range())
    {
        if (checkCancellation(result, cancelled))
            return result;
        if (v1Verified[int(piece)] && v2Verified[int(piece)])
        {
            ++result.validPieces;
            for (const lt::file_slice &slice : target.map_block(piece, 0, target.piece_size(piece)))
            {
                if (!files.pad_file_at(slice.file_index))
                {
                    result.files[fileReports[int(slice.file_index)]].verifiedBytes += slice.size;
                    result.verifiedBytes += slice.size;
                }
            }
        }
        else
        {
            ++result.unverifiedPieces;
        }
    }
    return result;
}

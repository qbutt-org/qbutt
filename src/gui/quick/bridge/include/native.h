#ifndef QBT_QUICK_NATIVE_H
#define QBT_QUICK_NATIVE_H

#include <cstdint>
#include <memory>

#include <QString>

#include "rust/cxx.h"

namespace Qbutt
{
    class WorkModel;
    class VariantModel;
    class ChapterModel;
    struct RetiredReaderCache;
    struct Services;

    std::unique_ptr<WorkModel> createWorkModel();
    std::unique_ptr<VariantModel> createVariantModel();
    std::unique_ptr<ChapterModel> createChapterModel();
    QString contentError(std::int32_t operation);
    void logContentError(const QString &operation, const QString &detail);
    QString formatSize(std::uint64_t bytes);
    QString contentLanguage();
    QString formatTranslationLabel(const QString &language, const QString &translation);
    QString readerError(std::int32_t operation);
    QString chapterTitle(const QString &number, const QString &volume, const QString &title);
    QString validatedImageUrl(const QString &path);
    void retireReaderCache(::rust::Box<RetiredReaderCache> cache);
    void retireContentServices(::rust::Box<Services> services);
}

#endif

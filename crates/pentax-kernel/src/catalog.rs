// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: typed, exact scoped registrations; see NOTICE.

/// Pinned libkernel functions; availability is decided by actual dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    CreateEqueue,
    DeleteEqueue,
    WaitEqueue,
    GetEventUserData,
    GetEventId,
    GetEventFilter,
    GetEventData,
    GetEventFflags,
    GetEventError,
    AddUserEvent,
    AddUserEventEdge,
    TriggerUserEvent,
    DeleteUserEvent,
    AddTimerEvent,
    AddHRTimerEvent,
    DeleteHRTimerEvent,
    AmprConstruct,
    AmprAprConstruct,
    AmprAprDestruct,
    AmprDestruct,
    AmprSetBuffer,
    AmprReset,
    AmprClearBuffer,
    AmprGetType,
    AmprGetOffset,
    AmprGetNum,
    AmprGetSize,
    AmprGetBase,
    CommonDialogInitialize,
    CommonDialogIsUsed,
    AppContentInitialize,
    SystemReceiveEvent,
    SystemGetStatus,
    UserInitialize,
    UserInitialize2,
    UserGetInitial,
    UserGetEvent,
    UserGetLoginList,
    UserGetName,
    UserGetNumber,
    UserGetGamePresets,
    UserGetAgeLevel,
    UserGetChatTranscription,
    UserGetPressAndHoldDelay,
    UserGetVibration,
    UserGetTriggerEffect,
    UserGetZoomEnabled,
    UserGetZoomFollowFocus,
    NpGetId,
    NpGetOnlineId,
    NpGetAccountId,
    NpGetAccountCountry,
    NpGetAccountAge,
    NpGetAccountLanguage,
    NpGetState,
    NpGetReachabilityState,
    NpHasSignedUp,
    NpCreateRequest,
    NpCreateAsyncRequest,
    NpDeleteRequest,
    NpAbortRequest,
    NpCheckAvailability,
    NpCheckReachability,
    NpPollAsync,
    NpCheckPremium,
    SaveInitialize,
    SaveTerminate,
    SaveCreateTransaction,
    SaveDeleteTransaction,
    SavePrepare,
    SaveCommit,
    SaveGetEvent,
    SaveMemorySetup,
    SaveMemoryGet,
    SaveMemorySet,
    SaveMemorySync,
    FiberInitialize,
    FiberInitializeInternal,
    FiberOptParamInitialize,
    FiberFinalize,
    FiberGetInfo,
    FiberRename,
    Open,
    Close,
    Read,
    Pread,
    Lseek,
    Fstat,
    Stat,
    CheckReachability,
    AllocateDirect,
    AllocateMainDirect,
    ReleaseDirect,
    DirectMemorySize,
    MapDirect,
    MapFlexible,
    MapNamedFlexible,
    Munmap,
    Mprotect,
    QueryProtection,
    CreateSema,
    DeleteSema,
    WaitSema,
    PollSema,
    SignalSema,
    CancelSema,
    CreateEventFlag,
    DeleteEventFlag,
    WaitEventFlag,
    PollEventFlag,
    SetEventFlag,
    ClearEventFlag,
    CancelEventFlag,
    ClockGettime,
    ClockGetres,
    Gettimeofday,
    ProcessTime,
    ProcessTimeCounter,
    ProcessTimeCounterFrequency,
    Usleep,
    Sleep,
    Nanosleep,
    Getpid,
    PthreadSelf,
    PthreadGetthreadid,
    PthreadSetaffinity,
    PthreadGetaffinity,
    PthreadEqual,
    SetGpo,
    GetGpi,
    PthreadCreate,
    PthreadJoin,
    PthreadExit,
    PthreadDetach,
    GetProcParam,
    Dlsym,
    LoadStartModule,
    TlsGetAddr,
    ReadTsc,
    TscFrequency,
    ErrnoLocation,
    PageSize,
    PosixOpen,
    PosixClose,
    PosixRead,
    PosixPread,
    PosixLseek,
    PosixFstat,
    PosixStat,
    PosixClockGettime,
    PosixClockGetres,
    PosixNanosleep,
    Getdirentries,
    Getdents,
    MutexAttrInit,
    MutexAttrDestroy,
    MutexAttrSetType,
    MutexAttrSetProtocol,
    MutexInit,
    MutexDestroy,
    MutexLock,
    MutexTryLock,
    MutexTimedLock,
    MutexUnlock,
    PosixMutexAttrInit,
    PosixMutexAttrDestroy,
    PosixMutexAttrSetType,
    PosixMutexAttrSetProtocol,
    PosixMutexInit,
    PosixMutexDestroy,
    PosixMutexLock,
    PosixMutexTryLock,
    PosixMutexTimedLock,
    PosixMutexUnlock,
    KeyCreate,
    KeyDelete,
    SetSpecific,
    GetSpecific,
    PosixKeyCreate,
    PosixKeyDelete,
    PosixSetSpecific,
    PthreadOnce,
    SetThreadDtors,
    SetThreadAtexitCount,
    SetThreadAtexitReport,
    SetApplicationHeapApi,
    MapNamedDirect,
    IsAddressSanitizerEnabled,
    CondAttrInit,
    CondAttrDestroy,
    CondAttrSetClock,
    CondInit,
    CondDestroy,
    CondSignal,
    CondBroadcast,
    CondWait,
    PosixCondWait,
    PosixCondAttrInit,
    PosixCondAttrDestroy,
    PosixCondAttrSetClock,
    PosixCondInit,
    PosixCondSignal,
    PosixCondBroadcast,
    RwlockInit,
    RwlockDestroy,
    RwlockReadLock,
    RwlockWriteLock,
    RwlockTryReadLock,
    RwlockTryWriteLock,
    RwlockTimedReadLock,
    RwlockTimedWriteLock,
    RwlockUnlock,
    RwlockAttrInit,
    RwlockAttrDestroy,
    RwlockAttrSetType,
    PosixRwlockInit,
    PosixRwlockDestroy,
    PosixRwlockWriteLock,
    ThreadAttrDestroy,
    ThreadAttrGet,
    ThreadAttrGetAffinity,
    ThreadAttrInit,
    ThreadAttrGetDetachState,
    ThreadAttrGetStack,
    ThreadAttrGetStackAddr,
    ThreadAttrGetStackSize,
    ThreadAttrGetGuardSize,
    ThreadAttrGetSoloSched,
    ThreadAttrSetStack,
    ThreadAttrSetStackAddr,
    ThreadAttrSetStackSize,
    ThreadAttrSetDetachState,
    ThreadAttrSetGuardSize,
    ThreadAttrSetInheritSched,
    ThreadAttrSetSchedParam,
    ThreadAttrSetSchedPolicy,
    ThreadAttrSetSoloSched,
    ThreadAttrSetAffinity,
    ThreadAttrGetSchedParam,
    ThreadAttrGetSchedPolicy,
    PosixThreadAttrInit,
    PosixThreadAttrDestroy,
    PosixThreadAttrGetStack,
    PosixThreadAttrSetStack,
    PosixThreadAttrSetStackSize,
    PosixThreadAttrGet,
    PosixThreadAttrGetSchedPolicy,
    PosixThreadAttrSetSchedPolicy,
    PosixThreadAttrSetDetachState,
    PosixThreadAttrSetSchedParam,
    PosixThreadAttrSetInheritSched,
    PosixThreadAttrGetStackSize,
    PosixThreadAttrGetDetachState,
    PosixThreadAttrSetGuardSize,
    PosixThreadAttrGetGuardSize,
    PosixThreadAttrGetSchedParam,
    PosixPthreadCreate,
    PosixPthreadCreateName,
    Write,
    PosixWrite,
    SystemParamGetInt,
    Gettimezone,
    ConvertUtcToLocaltime,
    ConvertLocaltimeToUtc,
    SysmoduleLoad,
    SysmoduleUnload,
    SysmoduleIsLoaded,
    SysmoduleLoadInternal,
    PosixSemInit,
    PosixSemDestroy,
    PosixSemWait,
    PosixSemTryWait,
    PosixSemRelativeWait,
    PosixSemTimedWait,
    PosixSemPost,
    PosixSemGetValue,
    PthreadSemInit,
    PthreadSemDestroy,
    PthreadSemWait,
    PthreadSemTryWait,
    PthreadSemTimedWait,
    PthreadSemPost,
    PthreadSemGetValue,
    SetVirtualRangeName,
    ClearVirtualRangeName,
    PadInit,
    PadOpen,
    PadGetHandle,
    PadClose,
    PadIsRemote,
    IsTrinityMode,
    GetOperationMode,
    SystemGetHdrToneMapLuminance,
    SystemGetDisplaySafeAreaInfo,
    AprResolveFilepathsToIdsAndFileSizes,
    AmprReadFile,
    AmprMeasureReadFile,
    AprSubmitCommandBufferAndGetResult,
    AprSubmitCommandBuffer,
    AprSubmitCommandBufferAndGetId,
    AprWaitCommandBuffer,
    AppContentAppParamGetInt,
    NetCtlInit,
    NetCtlTerm,
    NetCtlGetState,
    NetCtlGetStateV6,
    NetCtlGetNatInfo,
    NetCtlRegisterCallback,
    NetCtlUnregisterCallback,
    NetCtlCheckCallback,
    NetCtlGetResult,
    NetCtlGetInfo,
    RudpInit,
    RudpEnableInternalIOThread,
    RudpSetEventHandler,
    NetInit,
    NetGetErrorAddr,
    NetPoolCreate,
    NetPoolDestroy,
    NetResolverCreate,
    NetResolverDestroy,
    NetResolverAbort,
    SslInit,
    SslTerm,
    SslGetCaCerts,
    SslFreeCaCerts,
    HttpInit,
    HttpTerm,
    Http2Init,
    Http2Term,
    Http2CreateTemplate,
    Http2DeleteTemplate,
    Http2CreateRequestWithURL,
    Http2DeleteRequest,
    Http2AddRequestHeader,
    Http2SetRequestContentLength,
    Http2SetAuthEnabled,
    Http2SetAutoRedirect,
    Http2SetInflateGZIPEnabled,
    Http2SslDisableOption,
    Http2SslEnableOption,
    Http2SetRedirectCallback,
    Http2SetSslCallback,
    Http2SetRecvTimeOut,
    Http2SetSendTimeOut,
    Http2SetConnectTimeOut,
    Http2SetConnectionWaitTimeOut,
    Http2SetResolveTimeOut,
    Http2SetTimeOut,
    NpWebApi2Initialize,
    NpWebApi2GetMemoryPoolStats,
    NpWebApi2PushEventCreateHandle,
    NpWebApi2CreateUserContext,
    NpWebApi2DeleteUserContext,
    NpWebApi2CreateRequest,
    NpWebApi2SendRequest,
    NpWebApi2ReadData,
    NpWebApi2AbortRequest,
    NpWebApi2DeleteRequest,
    NpWebApi2AddHttpRequestHeader,
    NpWebApi2GetHttpResponseHeaderValueLength,
    NpWebApi2GetHttpResponseHeaderValue,
    NpWebApi2PushEventDeleteHandle,
    NpWebApi2PushEventDeletePushContext,
    NpWebApi2PushEventCreateFilter,
    NpWebApi2PushEventRegisterCallback,
    NpWebApi2CheckTimeout,
    NpWebApi2Terminate,
    MspaceCreate,
    MspaceDestroy,
    MspaceMalloc,
    MspaceFree,
    MspaceCalloc,
    MspaceRealloc,
    MspacePosixMemalign,
    MspaceMemalign,
    MspaceAlignedAlloc,
    MspaceReallocalign,
    MspaceMallocStats,
    MspaceMallocStatsFast,
    MspaceMallocUsableSize,
    PafMspaceCreate,
    PafMspaceDestroy,
    PafMspaceMalloc,
    PafMspaceMemalign,
    PafMspaceFree,
    PafMspaceCalloc,
    PafMspaceRealloc,
    PafMspaceReallocalign,
    PafMspaceMallocStats,
    PafMspaceMallocStatsFast,
    GameIntentInitialize,
    GameIntentTerminate,
    GameIntentReceiveIntent,
    GameIntentGetPropertyValueString,
    Trophy2CreateContext,
    Trophy2CreateHandle,
    Trophy2RegisterContext,
    Trophy2GetGameInfo,
    Trophy2GetGroupInfo,
    Trophy2GetGroupInfoArray,
    Trophy2GetTrophyInfo,
    Trophy2GetTrophyInfoArray,
    Trophy2GetGameIcon,
    Trophy2GetGroupIcon,
    Trophy2GetTrophyIcon,
    Trophy2RegisterUnlockCallback,
    Trophy2AbortHandle,
    Trophy2DestroyHandle,
    Trophy2DestroyContext,
    UdsInitialize,
    UdsCreateContext,
    UdsCreateHandle,
    UdsCreateEvent,
    UdsPostEvent,
    UdsDestroyHandle,
    UdsAbortHandle,
    UdsDestroyContext,
    UdsGetMemoryStat,
    UdsEventEstimateSize,
    UdsEventToString,
    UdsDestroyEvent,
    UdsRegisterContext,
    UdsCreateEventPropertyObject,
    UdsDestroyEventPropertyObject,
    UdsEventPropertyObjectSetString,
    UdsEventPropertyObjectSetInt32,
    UdsEventPropertyObjectSetUInt32,
    UdsEventPropertyObjectSetInt64,
    UdsEventPropertyObjectSetUInt64,
    UdsEventPropertyObjectSetFloat32,
    UdsEventPropertyObjectSetFloat64,
    UdsEventPropertyObjectSetBool,
    UdsEventPropertyObjectSetBinary,
    UdsEventPropertyObjectSetObject,
    UdsEventPropertyObjectSetArray,
    UdsCreateEventPropertyArray,
    UdsDestroyEventPropertyArray,
    UdsEventPropertyArraySetString,
    UdsEventPropertyArraySetInt32,
    UdsEventPropertyArraySetUInt32,
    UdsEventPropertyArraySetInt64,
    UdsEventPropertyArraySetUInt64,
    UdsEventPropertyArraySetFloat32,
    UdsEventPropertyArraySetFloat64,
    UdsEventPropertyArraySetBool,
    UdsEventPropertyArraySetBinary,
    UdsEventPropertyArraySetObject,
    UdsEventPropertyArraySetArray,
    UdsGetStorageStat,
    UdsTerminate,
    EntitlementAccessInitialize,
    EntitlementAccessGetSkuFlag,
    EntitlementAccessGetAddcontEntitlementInfoList,
    EntitlementAccessGetAddcontEntitlementInfo,
    EntitlementAccessGetEntitlementKey,
    FontAttachDeviceCacheBuffer,
    FontCloseFont,
    FontCreateRendererWithEdition,
    FontDestroyRenderer,
    FontBindRenderer,
    FontRebindRenderer,
    FontUnbindRenderer,
    FontSetScalePixel,
    FontSetEffectSlant,
    FontSetEffectWeight,
    FontSetupRenderScalePixel,
    FontSetupRenderEffectSlant,
    FontSetupRenderEffectWeight,
    FontGetHorizontalLayout,
    FontGetVerticalLayout,
    FontDefineAttribute,
    FontCreateLibrary,
    FontCreateLibraryWithEdition,
    FontCreateString,
    FontCreateWritingLine,
    FontDestroyString,
    FontDestroyWritingLine,
    FontDeleteGlyph,
    FontDestroyLibrary,
    FontGenerateCharGlyph,
    FontGetCharGlyphMetrics,
    FontGetRenderCharGlyphMetrics,
    FontGlyphDefineAttribute,
    FontMemoryInit,
    FontMemoryTerm,
    FontOpenFontInstance,
    FontOpenFontMemory,
    FontOpenFontSet,
    FontRenderCharGlyphImageHorizontal,
    FontRenderSurfaceInit,
    FontRenderSurfaceSetScissor,
    FontStringGetTerminateCode,
    FontStringGetTerminateOrder,
    FontStringGetWritingForm,
    FontStringRefersRenderCharacters,
    FontStringRefersTextCharacters,
    FontTextSourceInit,
    FontTextSourceRewind,
    FontTextSourceSetDefaultFont,
    FontTextSourceSetWritingForm,
    FontCharacterGetBidiLevel,
    FontCharacterGetSyllableStringState,
    FontCharacterGetTextFontCode,
    FontCharacterGetTextOrder,
    FontCharacterLooksFormatCharacters,
    FontCharacterLooksWhiteSpace,
    FontCharacterRefersTextBack,
    FontCharacterRefersTextNext,
    FontCharactersRefersTextCodes,
    FontTextCodesStepBack,
    FontTextCodesStepNext,
    FontWritingGetRenderMetrics,
    FontWritingInit,
    FontWritingLineClear,
    FontWritingLineGetOrderingSpace,
    FontWritingLineGetRenderMetrics,
    FontWritingLineRefersRenderStep,
    FontWritingLineWritesOrder,
    FontWritingRefersRenderStep,
    FontWritingRefersRenderStepCharacter,
    FontWritingSetMaskInvisible,
    FontSupportExternalFonts,
    FontSupportSystemFonts,
    FontSelectLibraryFt,
    FontSelectRendererFt,
    ImeSetText,
    ImeSetCaret,
    ImeParamInit,
    ImeGetPanelSize,
    ImeOpen,
    ImeUpdate,
    ImeClose,
    ImeSetTextGeometry,
    ImeKeyboardOpen,
    ImeKeyboardClose,
    ImeKeyboardGetResourceId,
    ImeKeyboardGetInfo,
    ImeKeyboardSetMode,
    MouseClose,
    MouseInit,
    MouseOpen,
    MouseRead,
    KeyboardInit,
    KeyboardOpen,
    KeyboardClose,
    KeyboardReadState,
    KeyboardRead,
    KeyboardGetKey2Char,
    ShareInitialize,
    ShareFeaturePermit,
    ShareTerminate,
    ContentExportInit2,
    ContentSearchInit,
    ContentDeleteInitialize,
    ShareCaptureVideoClip,
    ShareFeatureProhibit,
    ShareSetContentParam,
    ShareOpenMenuForContent,
    ShareCaptureVideoClipExtended,
    ShareCaptureScreenshot,
    ShareCaptureScreenshotExtended,
    ShareUnregisterContentEventCallback,
    ShareSetContentParamForApplicationTitle,
    ShareGetCurrentStatus,
    ShareRegisterContentEventCallback,
    ShareSetScreenshotOverlayImage,
    ShareGetRunningStatus,
    ShareSetCaptureSource,
    PlayGoInitialize,
    PlayGoTerminate,
    PlayGoOpen,
    PlayGoClose,
    PlayGoGetLocus,
    PlayGoGetInstallSpeed,
    PlayGoSetInstallSpeed,
    PlayGoGetChunkId,
    PlayGoGetToDoList,
    PlayGoSetToDoList,
    PlayGoPrefetch,
    PlayGoGetEta,
    PlayGoGetLanguageMask,
    PlayGoGetProgress,
    PlayGoGetOptionalChunk,
    PlayGoPrefetchOptionalChunk,
    PlayGoGetInstallChunkId,
    PlayGoGetSupportedOptionalChunk,
    PthreadGetprio,
    PthreadSetprio,
    PosixSchedGetPriorityMax,
    PosixSchedGetPriorityMin,
    PosixThreadSetprio,
    PosixThreadGetSchedParam,
    PosixThreadSetSchedParam,
    SystemHideSplashScreen,
    SystemParamGetString,
    SystemGetNoticeScreenSkipFlag,
    SystemDisableNoticeScreenSkipFlagAutoSet,
    SystemSetNoticeScreenSkipFlag,
    SystemPowerTick,
    SystemReportAbnormalTermination,
    AcmContextCreate,
    AudioOut2Initialize,
    AudioOut2ContextResetParam,
    AudioOut2ContextQueryMemory,
    AudioOut2GetSpeakerInfo,
    AudioOut2ContextCreate,
    AudioOut2ContextDestroy,
    AudioOut2ContextAdvance,
    AudioOut2ContextPush,
    AudioOut2ContextGetQueueLevel,
    AudioOut2UserCreate,
    AudioOut2UserDestroy,
    AudioOut2PortCreate,
    AudioOut2PortDestroy,
    AudioOut2PortGetState,
    AudioOut2PortSetAttributes,
}

/// Pinned identities and their explicit libScePosix spelling aliases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Registration {
    pub service: Service,
    pub nid: &'static str,
    pub source_name: &'static str,
    pub arguments: u8,
    pub module: &'static str,
    pub module_version: (u16, u16),
    pub library: &'static str,
    pub library_version: u16,
}

macro_rules! registrations {
    ($(($service:ident, $nid:literal, $name:literal, $arguments:literal $(, $library:literal)?)),* $(,)?) => {
        const BASE_REGISTRATIONS: &[Registration] = &[$(Registration {
            service: Service::$service, nid: $nid, source_name: $name, arguments: $arguments,
            module: "libkernel", module_version: (1, 1), library: registrations!(@library $( $library )?), library_version: 1,
        }),*];
    };
    (@library) => { "libkernel" };
    (@library $library:literal) => { $library };
}

registrations![
    (Open, "1G3lF1Gg1k8", "KernelOpen", 3),
    (Close, "UK2Tl2DWUns", "KernelClose", 1),
    (Read, "Cg4srZ6TKbU", "KernelRead", 3),
    (Pread, "+r3rMFwItV4", "KernelPread", 4),
    (Lseek, "oib76F-12fk", "KernelLseek", 3),
    (Fstat, "kBwCPsYX-m4", "KernelFstat", 2),
    (Stat, "eV9wAD2riIA", "KernelStat", 2),
    (
        CheckReachability,
        "uWyW3v98sU4",
        "KernelCheckReachability",
        1
    ),
    (
        AllocateDirect,
        "rTXw65xmLIA",
        "KernelAllocateDirectMemory",
        6
    ),
    (
        AllocateMainDirect,
        "B+vc2AO2Zrc",
        "KernelAllocateMainDirectMemory",
        4
    ),
    (ReleaseDirect, "MBuItvba6z8", "KernelReleaseDirectMemory", 2),
    (
        DirectMemorySize,
        "pO96TwzOm5E",
        "KernelGetDirectMemorySize",
        0
    ),
    (MapDirect, "L-Q3LEjIbgA", "KernelMapDirectMemory", 6),
    (MapFlexible, "IWIBBdTHit4", "KernelMapFlexibleMemory", 4),
    (MapFlexible, "4h6F1LLbTiw", "KernelMapFlexibleMemory", 4),
    (
        MapNamedFlexible,
        "mL8NDH86iQI",
        "KernelMapNamedFlexibleMemory",
        5
    ),
    (Munmap, "cQke9UuBQOk", "KernelMunmap", 2),
    (Mprotect, "vSMAm3cxYTY", "KernelMprotect", 3),
    (
        QueryProtection,
        "WFcfL2lzido",
        "KernelQueryMemoryProtection",
        4
    ),
    (CreateSema, "188x57JYp0g", "KernelCreateSema", 6),
    (DeleteSema, "R1Jvn8bSCW8", "KernelDeleteSema", 1),
    (WaitSema, "Zxa0VhQVTsk", "KernelWaitSema", 3),
    (PollSema, "12wOHk8ywb0", "KernelPollSema", 2),
    (SignalSema, "4czppHBiriw", "KernelSignalSema", 2),
    (CancelSema, "4DM06U2BNEY", "KernelCancelSema", 3),
    (CreateEventFlag, "BpFoboUJoZU", "KernelCreateEventFlag", 5),
    (DeleteEventFlag, "8mql9OcQnd4", "KernelDeleteEventFlag", 1),
    (WaitEventFlag, "JTvBflhYazQ", "KernelWaitEventFlag", 5),
    (PollEventFlag, "9lvj5DjHZiA", "KernelPollEventFlag", 4),
    (SetEventFlag, "IOnSvHzqu6A", "KernelSetEventFlag", 2),
    (ClearEventFlag, "7uhBFWRAS60", "KernelClearEventFlag", 2),
    (CancelEventFlag, "PZku4ZrXJqg", "KernelCancelEventFlag", 3),
    (ClockGettime, "QBi7HCK03hw", "KernelClockGettime", 2),
    (ClockGetres, "wRYVA5Zolso", "KernelClockGetres", 2),
    (Gettimeofday, "ejekcaNQNq0", "KernelGettimeofday", 1),
    (ProcessTime, "4J2sUJmuHZQ", "KernelGetProcessTime", 0),
    (
        ProcessTimeCounter,
        "fgxnMeTNUtY",
        "KernelGetProcessTimeCounter",
        0
    ),
    (
        ProcessTimeCounterFrequency,
        "BNowx2l588E",
        "KernelGetProcessTimeCounterFrequency",
        0
    ),
    (Usleep, "1jfXLRVzisc", "KernelUsleep", 1),
    (Usleep, "QcteRwbsnV0", "KernelUsleep", 1),
    (Sleep, "-ZR+hG7aDHw", "KernelSleep", 1),
    (Nanosleep, "QvsZxomvUHs", "KernelNanosleep", 2),
    (Getpid, "HoLVWNanBBc", "getpid", 0),
    (PthreadSelf, "aI+OeCz8xrQ", "PthreadSelf", 0),
    (PthreadSelf, "EotR8a3ASf4", "PthreadSelf", 0),
    (PthreadGetthreadid, "EI-5-jlq2dE", "PthreadGetthreadid", 0),
    (PthreadEqual, "3PtV6p3QNX4", "PthreadEqual", 2),
    (SetGpo, "ca7v6Cxulzs", "KernelSetGPO", 1),
    (GetGpi, "4oXYe9Xmk0Q", "KernelGetGPI", 0),
    (PthreadCreate, "6UgtwV+0zb4", "PthreadCreate", 5),
    (PthreadJoin, "onNY9Byn-W8", "PthreadJoin", 2),
    (PthreadDetach, "4qGrR6eoP9Y", "PthreadDetach", 1),
    (PthreadExit, "3kg7rT0NQIs", "PthreadExit", 1),
    (GetProcParam, "959qrazPIrg", "KernelGetProcParam", 0),
    (Dlsym, "LwG8g3niqwA", "KernelDlsym", 3),
    (LoadStartModule, "wzvqT4UqKX8", "KernelLoadStartModule", 6),
    (TlsGetAddr, "vNe1w4diLCs", "tls_get_addr", 1),
    (ReadTsc, "-2IRUCO--PM", "KernelReadTsc", 0),
    (TscFrequency, "1j3S3n-tTW4", "KernelGetTscFrequency", 0),
    (ErrnoLocation, "9BcDykPmo1I", "get_error_addr", 0),
    (PageSize, "k+AXqu2-eBc", "getpagesize", 0),
    (PosixOpen, "6c3rCVE-fTU", "open", 3),
    (PosixClose, "NNtFaKJbPt0", "close", 1),
    (PosixRead, "DRuBt2pvICk", "read", 3),
    (PosixRead, "AqBioC2vF3I", "read", 3),
    (PosixPread, "ezv-RSBNKqI", "pread", 4),
    (PosixLseek, "Oy6IpwgtYOk", "lseek", 3),
    (PosixFstat, "mqQMh1zPPT8", "fstat", 2),
    (PosixClockGettime, "lLMT9vJAck0", "clock_gettime", 2),
    (PageSize, "k+AXqu2-eBc", "getpagesize", 0, "Posix"),
    (
        PosixClockGettime,
        "lLMT9vJAck0",
        "clock_gettime",
        2,
        "Posix"
    ),
    (PosixClockGetres, "smIj7eqzZE8", "clock_getres", 2, "Posix"),
    (PosixNanosleep, "NhpspxdjEKU", "nanosleep", 2, "Posix"),
    (PosixNanosleep, "yS8U2TGCe1A", "nanosleep", 2, "Posix"),
    (PosixStat, "E6ao34wPw+U", "stat", 2, "Posix"),
    (PosixRead, "AqBioC2vF3I", "read", 3, "Posix"),
    (Getdirentries, "taRWhTJFTgE", "KernelGetdirentries", 4),
    (Getdents, "j2AIqSqJP0w", "KernelGetdents", 3),
    (MutexLock, "9UK1vLZQft4", "PthreadMutexLock", 1),
    (MutexUnlock, "tn3VlD0hG60", "PthreadMutexUnlock", 1),
    (MutexDestroy, "2Of0f+3mhhE", "PthreadMutexDestroy", 1),
    (MutexInit, "cmo1RIYva9o", "PthreadMutexInit", 3),
    (MutexTryLock, "upoVrzMHFeE", "PthreadMutexTrylock", 1),
    (
        MutexAttrDestroy,
        "smWEktiyyG0",
        "PthreadMutexattrDestroy",
        1
    ),
    (MutexAttrInit, "F8bUHwAG284", "PthreadMutexattrInit", 1),
    (
        MutexAttrSetType,
        "iMp8QpE+XO4",
        "PthreadMutexattrSettype",
        2
    ),
    (
        MutexAttrSetProtocol,
        "1FGvU0i9saQ",
        "PthreadMutexattrSetprotocol",
        2
    ),
    (MutexTimedLock, "IafI2PxcPnQ", "PthreadMutexTimedlock", 2),
    (PosixMutexLock, "7H0iTOciTLo", "pthread_mutex_lock", 1),
    (PosixMutexUnlock, "2Z+PpY6CaJg", "pthread_mutex_unlock", 1),
    (
        PosixMutexTimedLock,
        "Io9+nTKXZtA",
        "pthread_mutex_timedlock",
        2
    ),
    (
        PosixMutexLock,
        "7H0iTOciTLo",
        "pthread_mutex_lock",
        1,
        "Posix"
    ),
    (
        PosixMutexTryLock,
        "K-jXhbt2gn4",
        "pthread_mutex_trylock",
        1,
        "Posix"
    ),
    (
        PosixMutexUnlock,
        "2Z+PpY6CaJg",
        "pthread_mutex_unlock",
        1,
        "Posix"
    ),
    (
        PosixMutexInit,
        "ttHNfU+qDBU",
        "pthread_mutex_init",
        2,
        "Posix"
    ),
    (
        PosixMutexAttrInit,
        "dQHWEsJtoE4",
        "pthread_mutexattr_init",
        1,
        "Posix"
    ),
    (
        PosixMutexAttrSetType,
        "mDmgMOGVUqg",
        "pthread_mutexattr_settype",
        2,
        "Posix"
    ),
    (
        PosixMutexAttrSetProtocol,
        "5txKfcMUAok",
        "pthread_mutexattr_setprotocol",
        2,
        "Posix"
    ),
    (
        PosixMutexAttrDestroy,
        "HF7lK46xzjY",
        "pthread_mutexattr_destroy",
        1,
        "Posix"
    ),
    (
        PosixMutexDestroy,
        "ltCfaGr2JGE",
        "pthread_mutex_destroy",
        1,
        "Posix"
    ),
    (KeyCreate, "geDaqgH9lTg", "PthreadKeyCreate", 2),
    (KeyDelete, "PrdHuuDekhY", "PthreadKeyDelete", 1),
    (SetSpecific, "+BzXYkqYeLE", "PthreadSetspecific", 2),
    (GetSpecific, "eoht7mQOCmo", "PthreadGetspecific", 1),
    (
        PosixKeyCreate,
        "mqULNdimTn0",
        "pthread_key_create",
        2,
        "Posix"
    ),
    (
        PosixKeyDelete,
        "6BpEZuDT7YI",
        "pthread_key_delete",
        1,
        "Posix"
    ),
    (
        PosixSetSpecific,
        "WrOLvHU0yQM",
        "pthread_setspecific",
        2,
        "Posix"
    ),
    (
        GetSpecific,
        "0-KXaS70xy4",
        "pthread_getspecific",
        1,
        "Posix"
    ),
    (PthreadOnce, "Z4QosVuAsA0", "pthread_once", 2, "Posix"),
    (SetThreadDtors, "rNhWz+lvOMU", "KernelSetThreadDtors", 1),
    (
        SetThreadAtexitCount,
        "pB-yGZ2nQ9o",
        "KernelSetThreadAtexitCount",
        1
    ),
    (
        SetThreadAtexitReport,
        "WhCc1w3EhSI",
        "KernelSetThreadAtexitReport",
        1
    ),
    (
        SetApplicationHeapApi,
        "p5EcQeEeJAE",
        "KernelRtldSetApplicationHeapAPI",
        1
    ),
    (
        MapNamedDirect,
        "NcaWUxfMNIQ",
        "KernelMapNamedDirectMemory",
        7
    ),
    (
        IsAddressSanitizerEnabled,
        "jh+8XiK4LeE",
        "KernelIsAddressSanitizerEnabled",
        0
    ),
    (CondInit, "2Tb92quprl0", "PthreadCondInit", 3),
    (CondDestroy, "g+PZd2hiacg", "PthreadCondDestroy", 1),
    (CondDestroy, "RXXqi4CtF8w", "PthreadCondDestroy", 1),
    (CondBroadcast, "JGgj7Uvrl+A", "PthreadCondBroadcast", 1),
    (CondSignal, "kDh-NfxgMtE", "PthreadCondSignal", 1),
    (CondSignal, "2MOy+rUfuhQ", "PthreadCondSignal", 1),
    (CondAttrInit, "m5-2bsNfv7s", "PthreadCondattrInit", 1),
    (CondAttrDestroy, "waPcxYiR3WA", "PthreadCondattrDestroy", 1),
    (PosixCondAttrInit, "mKoTx03HRWA", "pthread_condattr_init", 1),
    (
        PosixCondAttrDestroy,
        "dJcuQVn6-Iw",
        "pthread_condattr_destroy",
        1
    ),
    (
        PosixCondAttrSetClock,
        "EjllaAqAPZo",
        "pthread_condattr_setclock",
        2
    ),
    (
        PosixCondBroadcast,
        "mkx2fVhNMsg",
        "pthread_cond_broadcast",
        1
    ),
    (
        PosixCondBroadcast,
        "mkx2fVhNMsg",
        "pthread_cond_broadcast",
        1,
        "Posix"
    ),
    (
        PosixCondSignal,
        "2MOy+rUfuhQ",
        "pthread_cond_signal",
        1,
        "Posix"
    ),
    (
        PosixCondInit,
        "0TyVk4MSLt0",
        "pthread_cond_init",
        2,
        "Posix"
    ),
    (
        PosixRwlockDestroy,
        "1471ajPzxh0",
        "pthread_rwlock_destroy",
        1,
        "Posix"
    ),
    (
        PosixRwlockInit,
        "ytQULN-nhL4",
        "pthread_rwlock_init",
        2,
        "Posix"
    ),
    (
        PosixRwlockWriteLock,
        "sIlRvQqsN2Y",
        "pthread_rwlock_wrlock",
        1,
        "Posix"
    ),
    (
        RwlockTryReadLock,
        "SFxTMOfuCkE",
        "PthreadRwlockTryrdlock",
        1,
        "Posix"
    ),
    (
        RwlockTryWriteLock,
        "XhWHn6P5R7U",
        "PthreadRwlockTrywrlock",
        1,
        "Posix"
    ),
    (RwlockInit, "6ULAa0fq4jA", "PthreadRwlockInit", 3),
    (RwlockDestroy, "BB+kb08Tl9A", "PthreadRwlockDestroy", 1),
    (RwlockReadLock, "Ox9i0c7L5w0", "PthreadRwlockRdlock", 1),
    (RwlockReadLock, "iGjsr1WAtI0", "PthreadRwlockRdlock", 1),
    (RwlockUnlock, "+L98PIbGttk", "PthreadRwlockUnlock", 1),
    (RwlockUnlock, "EgmLo6EWgso", "PthreadRwlockUnlock", 1),
    (RwlockWriteLock, "mqdNorrB+gI", "PthreadRwlockWrlock", 1),
    (RwlockWriteLock, "sIlRvQqsN2Y", "PthreadRwlockWrlock", 1),
    (
        RwlockTryWriteLock,
        "bIHoZCTomsI",
        "PthreadRwlockTrywrlock",
        1
    ),
    (
        RwlockTryReadLock,
        "XD3mDeybCnk",
        "PthreadRwlockTryrdlock",
        1
    ),
    (
        RwlockTimedReadLock,
        "iPtZRWICjrM",
        "PthreadRwlockTimedrdlock",
        2
    ),
    (
        RwlockTimedWriteLock,
        "adh--6nIqTk",
        "PthreadRwlockTimedwrlock",
        2
    ),
    (
        RwlockAttrDestroy,
        "i2ifZ3fS2fo",
        "PthreadRwlockattrDestroy",
        1
    ),
    (RwlockAttrInit, "yOfGg-I1ZII", "PthreadRwlockattrInit", 1),
    (
        RwlockAttrDestroy,
        "qsdmgXjqSgk",
        "PthreadRwlockattrDestroy",
        1
    ),
    (RwlockAttrInit, "xFebsA4YsFI", "PthreadRwlockattrInit", 1),
    (
        RwlockAttrSetType,
        "h-OifiouBd8",
        "PthreadRwlockattrSettype",
        2
    ),
    (ThreadAttrDestroy, "62KCwEMmzcM", "PthreadAttrDestroy", 1),
    (ThreadAttrGet, "x1X76arYMxU", "PthreadAttrGet", 2),
    (
        ThreadAttrGetAffinity,
        "8+s5BzZjxSg",
        "PthreadAttrGetaffinity",
        2
    ),
    (ThreadAttrInit, "nsYoNRywwNg", "PthreadAttrInit", 1),
    (
        ThreadAttrGetDetachState,
        "JaRMy+QcpeU",
        "PthreadAttrGetdetachstate",
        2
    ),
    (ThreadAttrGetStack, "-quPa4SEJUw", "PthreadAttrGetstack", 3),
    (
        ThreadAttrGetStackAddr,
        "Ru36fiTtJzA",
        "PthreadAttrGetstackaddr",
        2
    ),
    (
        ThreadAttrGetStackSize,
        "-fA+7ZlGDQs",
        "PthreadAttrGetstacksize",
        2
    ),
    (
        ThreadAttrGetGuardSize,
        "txHtngJ+eyc",
        "PthreadAttrGetguardsize",
        2
    ),
    (
        ThreadAttrGetSoloSched,
        "9RnL-m0+diQ",
        "PthreadAttrGetsolosched",
        2
    ),
    (ThreadAttrSetStack, "Bvn74vj6oLo", "PthreadAttrSetstack", 3),
    (
        ThreadAttrSetStackAddr,
        "F+yfmduIBB8",
        "PthreadAttrSetstackaddr",
        2
    ),
    (
        ThreadAttrSetStackSize,
        "UTXzJbWhhTE",
        "PthreadAttrSetstacksize",
        2
    ),
    (
        ThreadAttrSetStackSize,
        "2Q0z6rnBrTE",
        "PthreadAttrSetstacksize",
        2
    ),
    (
        ThreadAttrSetDetachState,
        "-Wreprtu0Qs",
        "PthreadAttrSetdetachstate",
        2
    ),
    (
        ThreadAttrSetGuardSize,
        "El+cQ20DynU",
        "PthreadAttrSetguardsize",
        2
    ),
    (
        ThreadAttrSetInheritSched,
        "eXbUSpEaTsA",
        "PthreadAttrSetinheritsched",
        2
    ),
    (
        ThreadAttrSetSchedParam,
        "DzES9hQF4f4",
        "PthreadAttrSetschedparam",
        2
    ),
    (
        ThreadAttrSetSchedPolicy,
        "4+h9EzwKF4I",
        "PthreadAttrSetschedpolicy",
        2
    ),
    (
        ThreadAttrSetSoloSched,
        "Dk6FC-TI+7Q",
        "PthreadAttrSetsolosched",
        2
    ),
    (
        ThreadAttrSetAffinity,
        "3qxgM4ezETA",
        "PthreadAttrSetaffinity",
        2
    ),
    (
        ThreadAttrGetSchedParam,
        "FXPWHNk8Of0",
        "PthreadAttrGetschedparam",
        2
    ),
    (
        PosixThreadAttrInit,
        "wtkt-teR1so",
        "pthread_attr_init",
        1,
        "Posix"
    ),
    (
        PosixThreadAttrDestroy,
        "zHchY8ft5pk",
        "pthread_attr_destroy",
        1,
        "Posix"
    ),
    (
        PosixThreadAttrGetStack,
        "vQm4fDEsWi8",
        "pthread_attr_getstack",
        3,
        "Posix"
    ),
    (
        PosixThreadAttrSetStack,
        "-SrbXpGR1f0",
        "pthread_attr_setstack",
        3,
        "Posix"
    ),
    (
        PosixThreadAttrSetStackSize,
        "2Q0z6rnBrTE",
        "pthread_attr_setstacksize",
        2,
        "Posix"
    ),
    (
        PosixThreadAttrGet,
        "Ucsu-OK+els",
        "pthread_attr_get_np",
        2,
        "Posix"
    ),
    (
        PosixThreadAttrGetSchedPolicy,
        "RtLRV-pBTTY",
        "pthread_attr_getschedpolicy",
        2,
        "Posix"
    ),
    (
        PosixThreadAttrSetSchedPolicy,
        "JarMIy8kKEY",
        "pthread_attr_setschedpolicy",
        2,
        "Posix"
    ),
    (
        PosixThreadAttrSetDetachState,
        "E+tyo3lp5Lw",
        "pthread_attr_setdetachstate",
        2,
        "Posix"
    ),
    (
        PosixThreadAttrSetSchedParam,
        "euKRgm0Vn2M",
        "pthread_attr_setschedparam",
        2,
        "Posix"
    ),
    (
        PosixThreadAttrSetInheritSched,
        "7ZlAakEf0Qg",
        "pthread_attr_setinheritsched",
        2,
        "Posix"
    ),
    (
        PosixThreadAttrGetStackSize,
        "0qOtCR-ZHck",
        "pthread_attr_getstacksize",
        2,
        "Posix"
    ),
    (
        PosixThreadAttrGetDetachState,
        "VUT1ZSrHT0I",
        "pthread_attr_getdetachstate",
        2,
        "Posix"
    ),
    (
        PosixThreadAttrSetGuardSize,
        "JKyG3SWyA10",
        "pthread_attr_setguardsize",
        2,
        "Posix"
    ),
    (
        PosixThreadAttrGetGuardSize,
        "JNkVVsVDmOk",
        "pthread_attr_getguardsize",
        2,
        "Posix"
    ),
    (
        PosixThreadAttrGetSchedParam,
        "qlk9pSLsUmM",
        "pthread_attr_getschedparam",
        2,
        "Posix"
    ),
    (
        PosixThreadAttrGetSchedParam,
        "FXPWHNk8Of0",
        "pthread_attr_getschedparam",
        2,
        "Posix"
    ),
];

impl Service {
    #[must_use]
    pub const fn registrations() -> &'static [Registration] {
        REGISTRATIONS
    }
}

// Pinned SymbolDatabase::UpdateName removes a leading "libSce". Retain the
// exact resolver and declare only this known alternative library spelling.
// Append aliases so every existing marker/service index stays unchanged.
const fn is_posix(entry: &Registration) -> bool {
    matches!(entry.library.as_bytes(), [b'P', b'o', b's', b'i', b'x'])
}

const fn posix_count() -> usize {
    let mut count = 0;
    let mut index = 0;
    while index < BASE_REGISTRATIONS.len() {
        if is_posix(&BASE_REGISTRATIONS[index]) {
            count += 1;
        }
        index += 1;
    }
    count
}

const fn with_posix_spelling<const N: usize>() -> [Registration; N] {
    let mut entries = [BASE_REGISTRATIONS[0]; N];
    let mut index = 0;
    while index < BASE_REGISTRATIONS.len() {
        entries[index] = BASE_REGISTRATIONS[index];
        index += 1;
    }
    let mut output = index;
    index = 0;
    while index < BASE_REGISTRATIONS.len() {
        if is_posix(&BASE_REGISTRATIONS[index]) {
            entries[output] = Registration {
                library: "libScePosix",
                ..BASE_REGISTRATIONS[index]
            };
            output += 1;
        }
        index += 1;
    }
    // New exports follow the original aliases, preserving all existing IDs.
    index = 0;
    while index < THREAD_REGISTRATIONS.len() {
        entries[output] = THREAD_REGISTRATIONS[index];
        entries[output + THREAD_REGISTRATIONS.len()] = Registration {
            library: "libScePosix",
            ..THREAD_REGISTRATIONS[index]
        };
        output += 1;
        index += 1;
    }
    output += THREAD_REGISTRATIONS.len();
    index = 0;
    while index < OUTPUT_REGISTRATIONS.len() {
        entries[output] = OUTPUT_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < SYSTEM_REGISTRATIONS.len() {
        entries[output] = SYSTEM_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < CALENDAR_REGISTRATIONS.len() {
        entries[output] = CALENDAR_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < SYSMODULE_REGISTRATIONS.len() {
        entries[output] = SYSMODULE_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < SEMAPHORE_REGISTRATIONS.len() {
        entries[output] = SEMAPHORE_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < VIRTUAL_NAME_REGISTRATIONS.len() {
        entries[output] = VIRTUAL_NAME_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < CONDITION_WAIT_REGISTRATIONS.len() {
        entries[output] = CONDITION_WAIT_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < FIBER_REGISTRATIONS.len() {
        entries[output] = FIBER_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < DIALOG_REGISTRATIONS.len() {
        entries[output] = DIALOG_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < AFFINITY_REGISTRATIONS.len() {
        entries[output] = AFFINITY_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < CONTENT_EVENT_REGISTRATIONS.len() {
        entries[output] = CONTENT_EVENT_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < USER_REGISTRATIONS.len() {
        entries[output] = USER_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < NP_REGISTRATIONS.len() {
        entries[output] = NP_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < SAVE_REGISTRATIONS.len() {
        entries[output] = SAVE_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < AMPR_REGISTRATIONS.len() {
        entries[output] = AMPR_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < PAD_REGISTRATIONS.len() {
        entries[output] = PAD_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < HARDWARE_REGISTRATIONS.len() {
        entries[output] = HARDWARE_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < EVENT_QUEUE_REGISTRATIONS.len() {
        entries[output] = EVENT_QUEUE_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < SYSTEM_DISPLAY_REGISTRATIONS.len() {
        entries[output] = SYSTEM_DISPLAY_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < APR_PATH_REGISTRATIONS.len() {
        entries[output] = APR_PATH_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < AMPR_READ_REGISTRATIONS.len() {
        entries[output] = AMPR_READ_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < APR_SUBMIT_REGISTRATIONS.len() {
        entries[output] = APR_SUBMIT_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < CONTENT_PARAMETER_REGISTRATIONS.len() {
        entries[output] = CONTENT_PARAMETER_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < NET_CTL_REGISTRATIONS.len() {
        entries[output] = NET_CTL_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < RUDP_REGISTRATIONS.len() {
        entries[output] = RUDP_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < NETWORK_REGISTRATIONS.len() {
        entries[output] = NETWORK_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < SSL_LIFECYCLE_V2_REGISTRATIONS.len() {
        entries[output] = SSL_LIFECYCLE_V2_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < HTTP2_REGISTRATIONS.len() {
        entries[output] = HTTP2_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < NP_WEBAPI2_REGISTRATIONS.len() {
        entries[output] = NP_WEBAPI2_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < MSPACE_REGISTRATIONS.len() {
        entries[output] = MSPACE_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < GAME_INTENT_REGISTRATIONS.len() {
        entries[output] = GAME_INTENT_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < TROPHY2_REGISTRATIONS.len() {
        entries[output] = TROPHY2_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < UDS_REGISTRATIONS.len() {
        entries[output] = UDS_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < ENTITLEMENT_ACCESS_REGISTRATIONS.len() {
        entries[output] = ENTITLEMENT_ACCESS_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < FONT_REGISTRATIONS.len() {
        entries[output] = FONT_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < IME_REGISTRATIONS.len() {
        entries[output] = IME_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < RAW_INPUT_REGISTRATIONS.len() {
        entries[output] = RAW_INPUT_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < SHARE_REGISTRATIONS.len() {
        entries[output] = SHARE_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < CONTENT_INIT_REGISTRATIONS.len() {
        entries[output] = CONTENT_INIT_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < SHARE_MEDIA_REGISTRATIONS.len() {
        entries[output] = SHARE_MEDIA_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < PLAYGO_REGISTRATIONS.len() {
        entries[output] = PLAYGO_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < PRIORITY_REGISTRATIONS.len() {
        entries[output] = PRIORITY_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < SYSTEM_NOTICE_REGISTRATIONS.len() {
        entries[output] = SYSTEM_NOTICE_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < ACM_REGISTRATIONS.len() {
        entries[output] = ACM_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < AUDIO_OUT2_REGISTRATIONS.len() {
        entries[output] = AUDIO_OUT2_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    let mut index = 0;
    while index < AUDIO_OUT2_LIFECYCLE_REGISTRATIONS.len() {
        entries[output] = AUDIO_OUT2_LIFECYCLE_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    index = 0;
    while index < POSIX_SELF_REGISTRATIONS.len() {
        entries[output] = POSIX_SELF_REGISTRATIONS[index];
        output += 1;
        index += 1;
    }
    assert!(output == N);
    entries
}

const THREAD_REGISTRATIONS: &[Registration] = &[
    Registration {
        service: Service::PosixPthreadCreate,
        nid: "OxhIB8LB-PQ",
        source_name: "pthread_create",
        arguments: 4,
        library: "Posix",
        ..BASE_REGISTRATIONS[0]
    },
    Registration {
        service: Service::PosixPthreadCreateName,
        nid: "Jmi+9w9u0E4",
        source_name: "pthread_create_name_np",
        arguments: 5,
        library: "Posix",
        ..BASE_REGISTRATIONS[0]
    },
];
const AFFINITY_REGISTRATIONS: &[Registration] = &[
    Registration {
        service: Service::PthreadSetaffinity,
        nid: "bt3CTBKmGyI",
        source_name: "PthreadSetaffinity",
        arguments: 2,
        ..BASE_REGISTRATIONS[0]
    },
    Registration {
        service: Service::PthreadGetaffinity,
        nid: "rcrVFJsQWRY",
        source_name: "PthreadGetaffinity",
        arguments: 2,
        ..BASE_REGISTRATIONS[0]
    },
];
pub const REGISTRATIONS: &[Registration] = &with_posix_spelling::<
    {
        BASE_REGISTRATIONS.len()
            + posix_count()
            + 2 * THREAD_REGISTRATIONS.len()
            + OUTPUT_REGISTRATIONS.len()
            + SYSTEM_REGISTRATIONS.len()
            + CALENDAR_REGISTRATIONS.len()
            + SYSMODULE_REGISTRATIONS.len()
            + SEMAPHORE_REGISTRATIONS.len()
            + VIRTUAL_NAME_REGISTRATIONS.len()
            + CONDITION_WAIT_REGISTRATIONS.len()
            + FIBER_REGISTRATIONS.len()
            + DIALOG_REGISTRATIONS.len()
            + AFFINITY_REGISTRATIONS.len()
            + CONTENT_EVENT_REGISTRATIONS.len()
            + USER_REGISTRATIONS.len()
            + NP_REGISTRATIONS.len()
            + SAVE_REGISTRATIONS.len()
            + AMPR_REGISTRATIONS.len()
            + PAD_REGISTRATIONS.len()
            + HARDWARE_REGISTRATIONS.len()
            + EVENT_QUEUE_REGISTRATIONS.len()
            + SYSTEM_DISPLAY_REGISTRATIONS.len()
            + APR_PATH_REGISTRATIONS.len()
            + AMPR_READ_REGISTRATIONS.len()
            + APR_SUBMIT_REGISTRATIONS.len()
            + CONTENT_PARAMETER_REGISTRATIONS.len()
            + NET_CTL_REGISTRATIONS.len()
            + RUDP_REGISTRATIONS.len()
            + NETWORK_REGISTRATIONS.len()
            + SSL_LIFECYCLE_V2_REGISTRATIONS.len()
            + HTTP2_REGISTRATIONS.len()
            + NP_WEBAPI2_REGISTRATIONS.len()
            + MSPACE_REGISTRATIONS.len()
            + GAME_INTENT_REGISTRATIONS.len()
            + TROPHY2_REGISTRATIONS.len()
            + UDS_REGISTRATIONS.len()
            + ENTITLEMENT_ACCESS_REGISTRATIONS.len()
            + FONT_REGISTRATIONS.len()
            + IME_REGISTRATIONS.len()
            + RAW_INPUT_REGISTRATIONS.len()
            + SHARE_REGISTRATIONS.len()
            + CONTENT_INIT_REGISTRATIONS.len()
            + SHARE_MEDIA_REGISTRATIONS.len()
            + PLAYGO_REGISTRATIONS.len()
            + PRIORITY_REGISTRATIONS.len()
            + SYSTEM_NOTICE_REGISTRATIONS.len()
            + ACM_REGISTRATIONS.len()
            + AUDIO_OUT2_REGISTRATIONS.len()
            + AUDIO_OUT2_LIFECYCLE_REGISTRATIONS.len()
            + POSIX_SELF_REGISTRATIONS.len()
    },
>();

#[path = "catalog/font.rs"]
mod font;
use font::FONT_REGISTRATIONS;

#[path = "catalog/ime.rs"]
mod ime;
use ime::IME_REGISTRATIONS;

#[path = "catalog/raw_input.rs"]
mod raw_input;
use raw_input::RAW_INPUT_REGISTRATIONS;

#[path = "catalog/share.rs"]
mod share;
use share::SHARE_REGISTRATIONS;

#[path = "catalog/content_init.rs"]
mod content_init;
use content_init::CONTENT_INIT_REGISTRATIONS;

#[path = "catalog/share_media.rs"]
mod share_media;
use share_media::SHARE_MEDIA_REGISTRATIONS;

#[path = "catalog/playgo.rs"]
mod playgo;
use playgo::PLAYGO_REGISTRATIONS;

#[path = "catalog/priority.rs"]
mod priority;
use priority::PRIORITY_REGISTRATIONS;

#[path = "catalog/system_notice.rs"]
mod system_notice;
use system_notice::SYSTEM_NOTICE_REGISTRATIONS;

#[path = "catalog/acm.rs"]
mod acm;
use acm::ACM_REGISTRATIONS;

#[path = "catalog/audio_out2.rs"]
mod audio_out2;
use audio_out2::AUDIO_OUT2_LIFECYCLE_REGISTRATIONS;
use audio_out2::AUDIO_OUT2_REGISTRATIONS;

#[path = "catalog/posix_self.rs"]
mod posix_self;
use posix_self::POSIX_SELF_REGISTRATIONS;

// Pinned 209dffe libNet.cpp:1596..1737; only Initialize and List have a
// separately observed full-prefix import. No mixed or inferred aliases.
const ENTITLEMENT_ACCESS_REGISTRATIONS: [Registration; 7] = {
    use Service::*;
    let exports = [
        (
            EntitlementAccessInitialize,
            "jO8DM8oyego",
            "LibNpEntitlementAccess::NpEntitlementAccessInitialize",
            2,
        ),
        (
            EntitlementAccessGetSkuFlag,
            "lPDO62PpJIA",
            "LibNpEntitlementAccess::NpEntitlementAccessGetSkuFlag",
            1,
        ),
        (
            EntitlementAccessGetAddcontEntitlementInfoList,
            "TFyU+KFBv54",
            "LibNpEntitlementAccess::NpEntitlementAccessGetAddcontEntitlementInfoList",
            4,
        ),
        (
            EntitlementAccessGetAddcontEntitlementInfo,
            "xddD23+8TfQ",
            "LibNpEntitlementAccess::NpEntitlementAccessGetAddcontEntitlementInfo",
            3,
        ),
        (
            EntitlementAccessGetEntitlementKey,
            "5LiMEPuW0DQ",
            "LibNpEntitlementAccess::NpEntitlementAccessGetEntitlementKey",
            3,
        ),
    ];
    let mut entries = [Registration {
        service: EntitlementAccessInitialize,
        nid: "jO8DM8oyego",
        source_name: "LibNpEntitlementAccess::NpEntitlementAccessInitialize",
        arguments: 2,
        module: "NpEntitlementAccess",
        module_version: (1, 1),
        library: "NpEntitlementAccess",
        library_version: 1,
    }; 7];
    let mut index = 0;
    while index < exports.len() {
        let (service, nid, source_name, arguments) = exports[index];
        entries[index] = Registration {
            service,
            nid,
            source_name,
            arguments,
            ..entries[0]
        };
        index += 1;
    }
    entries[5] = Registration {
        module: "libSceNpEntitlementAccess",
        library: "libSceNpEntitlementAccess",
        ..entries[0]
    };
    entries[6] = Registration {
        module: "libSceNpEntitlementAccess",
        library: "libSceNpEntitlementAccess",
        ..entries[2]
    };
    entries
};

#[cfg(test)]
#[path = "catalog/entitlement_access.rs"]
mod entitlement_access_tests;

// Kept last in the generated catalogue so existing service markers stay stable.
const APR_PATH_REGISTRATIONS: &[Registration] = &[Registration {
    service: Service::AprResolveFilepathsToIdsAndFileSizes,
    nid: "gEpBkcwxUjw",
    source_name: "Apr::ResolveFilepathsToIdsAndFileSizes",
    arguments: 5,
    ..BASE_REGISTRATIONS[0]
}];

// PentaX compatibility bindings for the two observed version2.1 lifecycle
// imports. The pinned source declares version1.1 only. This deliberately
// reuses the reviewed one-argument ABI/owned state, not a wildcard SDK alias.
const SSL_LIFECYCLE_V2_REGISTRATIONS: [Registration; 2] = [
    Registration {
        service: Service::SslInit,
        nid: "hdpVEUDFW3s",
        source_name: "Ssl::SslInit",
        arguments: 1,
        module: "libSceSsl",
        module_version: (2, 1),
        library: "libSceSsl",
        ..BASE_REGISTRATIONS[0]
    },
    Registration {
        service: Service::SslTerm,
        nid: "0K1yQ6Lv-Yc",
        source_name: "Ssl::SslTerm",
        arguments: 1,
        module: "libSceSsl",
        module_version: (2, 1),
        library: "libSceSsl",
        ..BASE_REGISTRATIONS[0]
    },
];

const HTTP2_REGISTRATIONS: [Registration; 84] = {
    let exports = [
        (Service::Http2Init, "3JCe3lCbQ8A", "LibHttp2::Http2Init", 4),
        (Service::Http2Term, "YiBUtz-pGkc", "LibHttp2::Http2Term", 1),
        (
            Service::Http2CreateTemplate,
            "+wCt7fCijgk",
            "LibHttp2::Http2CreateTemplate",
            4,
        ),
        (
            Service::Http2DeleteTemplate,
            "pDom5-078DA",
            "LibHttp2::Http2DeleteTemplate",
            1,
        ),
        (
            Service::Http2CreateRequestWithURL,
            "mmyOCxQMVYQ",
            "LibHttp2::Http2CreateRequestWithURL",
            4,
        ),
        (
            Service::Http2DeleteRequest,
            "c8D9qIjo8EY",
            "LibHttp2::Http2DeleteRequest",
            1,
        ),
        (
            Service::Http2AddRequestHeader,
            "nrPfOE8TQu0",
            "LibHttp2::Http2AddRequestHeader",
            4,
        ),
        (
            Service::Http2SetRequestContentLength,
            "FSAFOzi0FpM",
            "LibHttp2::Http2SetRequestContentLength",
            2,
        ),
        (
            Service::Http2SetAuthEnabled,
            "jjFahkBPCYs",
            "LibHttp2::Http2SetAuthEnabled",
            2,
        ),
        (
            Service::Http2SetAutoRedirect,
            "b9AvoIaOuHI",
            "LibHttp2::Http2SetAutoRedirect",
            2,
        ),
        (
            Service::Http2SetInflateGZIPEnabled,
            "uRosf8GQbHQ",
            "LibHttp2::Http2SetInflateGZIPEnabled",
            2,
        ),
        (
            Service::Http2SslDisableOption,
            "B37SruheQ5Y",
            "LibHttp2::Http2SslDisableOption",
            2,
        ),
        (
            Service::Http2SslEnableOption,
            "EWcwMpbr5F8",
            "LibHttp2::Http2SslEnableOption",
            2,
        ),
        (
            Service::Http2SetRedirectCallback,
            "BJgi0CH7al4",
            "LibHttp2::Http2SetRedirectCallback",
            3,
        ),
        (
            Service::Http2SetSslCallback,
            "YrWX+DhPHQY",
            "LibHttp2::Http2SetSslCallback",
            3,
        ),
        (
            Service::Http2SetRecvTimeOut,
            "izvHhqgDt44",
            "LibHttp2::Http2SetRecvTimeOut",
            2,
        ),
        (
            Service::Http2SetSendTimeOut,
            "XPtW45xiLHk",
            "LibHttp2::Http2SetSendTimeOut",
            2,
        ),
        (
            Service::Http2SetConnectTimeOut,
            "-HIO4VT87v8",
            "LibHttp2::Http2SetConnectTimeOut",
            2,
        ),
        (
            Service::Http2SetConnectionWaitTimeOut,
            "n8hMLe31OPA",
            "LibHttp2::Http2SetConnectionWaitTimeOut",
            2,
        ),
        (
            Service::Http2SetResolveTimeOut,
            "ACjtE27aErY",
            "LibHttp2::Http2SetResolveTimeOut",
            2,
        ),
        (
            Service::Http2SetTimeOut,
            "VYMxTcBqSE0",
            "LibHttp2::Http2SetTimeOut",
            2,
        ),
    ];
    let mut entries = [BASE_REGISTRATIONS[0]; 84];
    let mut index = 0;
    while index < exports.len() {
        let (service, nid, source_name, arguments) = exports[index];
        let entry = Registration {
            service,
            nid,
            source_name,
            arguments,
            module: "Http2",
            library: "Http2",
            ..BASE_REGISTRATIONS[0]
        };
        entries[index * 4] = entry;
        entries[index * 4 + 1] = Registration {
            module: "libSceHttp2",
            ..entry
        };
        entries[index * 4 + 2] = Registration {
            library: "libSceHttp2",
            ..entry
        };
        entries[index * 4 + 3] = Registration {
            module: "libSceHttp2",
            library: "libSceHttp2",
            ..entry
        };
        index += 1;
    }
    entries
};

// Public f186b357 libNet.cpp:2012,2356..2375. The pinned resolver strips
// one leading libSce independently from the module and library identities.
// Append after every retained group; declaration does not promise execution.
const NP_WEBAPI2_REGISTRATIONS: [Registration; 76] = {
    use Service::*;
    let exports = [
        (
            NpWebApi2Initialize,
            "+o9816YQhqQ",
            "LibNpWebApi2::NpWebApi2Initialize",
            2,
        ),
        (
            NpWebApi2GetMemoryPoolStats,
            "Xweb+naPZ8Y",
            "LibNpWebApi2::NpWebApi2GetMemoryPoolStats",
            2,
        ),
        (
            NpWebApi2PushEventCreateHandle,
            "WV1GwM32NgY",
            "LibNpWebApi2::NpWebApi2PushEventCreateHandle",
            1,
        ),
        (
            NpWebApi2CreateUserContext,
            "sk54bi6FtYM",
            "LibNpWebApi2::NpWebApi2CreateUserContext",
            2,
        ),
        (
            NpWebApi2DeleteUserContext,
            "9X9+cneTGUU",
            "LibNpWebApi2::NpWebApi2DeleteUserContext",
            1,
        ),
        (
            NpWebApi2CreateRequest,
            "3EI-OSJ65Xc",
            "LibNpWebApi2::NpWebApi2CreateRequest",
            6,
        ),
        (
            NpWebApi2SendRequest,
            "lQOCF84lvzw",
            "LibNpWebApi2::NpWebApi2SendRequest",
            4,
        ),
        (
            NpWebApi2ReadData,
            "OOY9+ObfKec",
            "LibNpWebApi2::NpWebApi2ReadData",
            3,
        ),
        (
            NpWebApi2AbortRequest,
            "zpiPsH7dbFQ",
            "LibNpWebApi2::NpWebApi2AbortRequest",
            1,
        ),
        (
            NpWebApi2DeleteRequest,
            "vvzWO-DvG1s",
            "LibNpWebApi2::NpWebApi2DeleteRequest",
            1,
        ),
        (
            NpWebApi2AddHttpRequestHeader,
            "egOOvrnF6mI",
            "LibNpWebApi2::NpWebApi2AddHttpRequestHeader",
            3,
        ),
        (
            NpWebApi2GetHttpResponseHeaderValueLength,
            "HwP3aM+c85c",
            "LibNpWebApi2::NpWebApi2GetHttpResponseHeaderValueLength",
            3,
        ),
        (
            NpWebApi2GetHttpResponseHeaderValue,
            "hksbskNToEA",
            "LibNpWebApi2::NpWebApi2GetHttpResponseHeaderValue",
            4,
        ),
        (
            NpWebApi2PushEventDeleteHandle,
            "fIATVMo4Y1w",
            "LibNpWebApi2::NpWebApi2PushEventDeleteHandle",
            2,
        ),
        (
            NpWebApi2PushEventDeletePushContext,
            "QafxeZM3WK4",
            "LibNpWebApi2::NpWebApi2PushEventDeletePushContext",
            2,
        ),
        (
            NpWebApi2PushEventCreateFilter,
            "MsaFhR+lPE4",
            "LibNpWebApi2::NpWebApi2PushEventCreateFilter",
            6,
        ),
        (
            NpWebApi2PushEventRegisterCallback,
            "fY3QqeNkF8k",
            "LibNpWebApi2::NpWebApi2PushEventRegisterCallback",
            4,
        ),
        (
            NpWebApi2CheckTimeout,
            "3Tt9zL3tkoc",
            "LibNpWebApi2::NpWebApi2CheckTimeout",
            0,
        ),
        (
            NpWebApi2Terminate,
            "bEvXpcEk200",
            "LibNpWebApi2::NpWebApi2Terminate",
            1,
        ),
    ];
    let mut entries = [BASE_REGISTRATIONS[0]; 76];
    let mut index = 0;
    while index < exports.len() {
        let (service, nid, source_name, arguments) = exports[index];
        let entry = Registration {
            service,
            nid,
            source_name,
            arguments,
            module: "NpWebApi2",
            library: "NpWebApi2",
            ..BASE_REGISTRATIONS[0]
        };
        entries[index * 4] = entry;
        entries[index * 4 + 1] = Registration {
            module: "libSceNpWebApi2",
            ..entry
        };
        entries[index * 4 + 2] = Registration {
            library: "libSceNpWebApi2",
            ..entry
        };
        entries[index * 4 + 3] = Registration {
            module: "libSceNpWebApi2",
            library: "libSceNpWebApi2",
            ..entry
        };
        index += 1;
    }
    entries
};

#[cfg(test)]
#[path = "catalog/np_webapi2.rs"]
mod np_webapi2_tests;

// AnyPS5 68323bae Mspace.cpp supplies these public definitions and its pinned
// NID generator supplies their names. The exact SCE scope below is the admitted
// PentaX compatibility profile; that source does not declare SCE descriptors.
// No canonical or mixed-prefix alias is inferred from the libc output filename.
const MSPACE_REGISTRATIONS: [Registration; 23] = {
    use Service::*;
    let exports = [
        (MspaceCreate, "-hn1tcVHq5Q", "sceLibcMspaceCreate", 4),
        (MspaceDestroy, "W6SiVSiCDtI", "sceLibcMspaceDestroy", 1),
        (MspaceMalloc, "OJjm-QOIHlI", "sceLibcMspaceMalloc", 2),
        (MspaceFree, "Vla-Z+eXlxo", "sceLibcMspaceFree", 2),
        (MspaceCalloc, "LYo3GhIlB38", "sceLibcMspaceCalloc", 3),
        (MspaceRealloc, "gigoVHZvVPE", "sceLibcMspaceRealloc", 3),
        (
            MspacePosixMemalign,
            "qWESlyXMI3E",
            "sceLibcMspacePosixMemalign",
            4,
        ),
        (MspaceMemalign, "iF1iQHzxBJU", "sceLibcMspaceMemalign", 3),
        (
            MspaceAlignedAlloc,
            "ljkqMcC4-mk",
            "sceLibcMspaceAlignedAlloc",
            3,
        ),
        (
            MspaceReallocalign,
            "p6lrRW8-MLY",
            "sceLibcMspaceReallocalign",
            4,
        ),
        (
            MspaceMallocStats,
            "mfHdJTIvhuo",
            "sceLibcMspaceMallocStats",
            2,
        ),
        (
            MspaceMallocStatsFast,
            "k04jLXu3+Ic",
            "sceLibcMspaceMallocStatsFast",
            2,
        ),
        (
            MspaceMallocUsableSize,
            "fEoW6BJsPt4",
            "sceLibcMspaceMallocUsableSize",
            1,
        ),
        (PafMspaceCreate, "6hdfGRKHefs", "sceLibcPafMspaceCreate", 5),
        (
            PafMspaceDestroy,
            "qB5nGjWa-bk",
            "sceLibcPafMspaceDestroy",
            1,
        ),
        (PafMspaceMalloc, "QuZzFJD5Hrw", "sceLibcPafMspaceMalloc", 3),
        (
            PafMspaceMemalign,
            "PKJcFUfhKtw",
            "sceLibcPafMspaceMemalign",
            4,
        ),
        (PafMspaceFree, "9mMuuhXMwqQ", "sceLibcPafMspaceFree", 3),
        (PafMspaceCalloc, "-lZdT34nAAE", "sceLibcPafMspaceCalloc", 4),
        (
            PafMspaceRealloc,
            "u32UXVridxQ",
            "sceLibcPafMspaceRealloc",
            4,
        ),
        (
            PafMspaceReallocalign,
            "4SvlEtd0j40",
            "sceLibcPafMspaceReallocalign",
            5,
        ),
        (
            PafMspaceMallocStats,
            "mO8NB8whKy8",
            "sceLibcPafMspaceMallocStats",
            2,
        ),
        (
            PafMspaceMallocStatsFast,
            "OmG3YPCBLJs",
            "sceLibcPafMspaceMallocStatsFast",
            2,
        ),
    ];
    let mut entries = [BASE_REGISTRATIONS[0]; 23];
    let mut index = 0;
    while index < exports.len() {
        let (service, nid, source_name, arguments) = exports[index];
        entries[index] = Registration {
            service,
            nid,
            source_name,
            arguments,
            module: "libSceLibcInternal",
            library: "libSceLibcInternal",
            ..BASE_REGISTRATIONS[0]
        };
        index += 1;
    }
    entries
};

#[cfg(test)]
#[path = "catalog/mspace.rs"]
mod mspace_tests;

// Pinned 209dffe libNet.cpp:1996..2081 declares the canonical scope;
// the separately confirmed prefixed pair is explicit, without mixed aliases.
const GAME_INTENT_REGISTRATIONS: [Registration; 8] = {
    use Service::*;
    let exports = [
        (
            GameIntentInitialize,
            "m87BHxt-H60",
            "LibNpGameIntent::NpGameIntentInitialize",
            1,
        ),
        (
            GameIntentTerminate,
            "0HBYxYAjmf0",
            "LibNpGameIntent::NpGameIntentTerminate",
            0,
        ),
        (
            GameIntentReceiveIntent,
            "jEIXUAr9XE8",
            "LibNpGameIntent::NpGameIntentReceiveIntent",
            1,
        ),
        (
            GameIntentGetPropertyValueString,
            "rPl0INNc-M8",
            "LibNpGameIntent::NpGameIntentGetPropertyValueString",
            4,
        ),
    ];
    let mut entries = [BASE_REGISTRATIONS[0]; 8];
    let mut index = 0;
    while index < exports.len() {
        let (service, nid, source_name, arguments) = exports[index];
        let canonical = Registration {
            service,
            nid,
            source_name,
            arguments,
            module: "NpGameIntent",
            library: "NpGameIntent",
            ..BASE_REGISTRATIONS[0]
        };
        entries[index * 2] = canonical;
        entries[index * 2 + 1] = Registration {
            module: "libSceNpGameIntent",
            library: "libSceNpGameIntent",
            ..canonical
        };
        index += 1;
    }
    entries
};

#[cfg(test)]
#[path = "catalog/game_intent.rs"]
mod game_intent_tests;

// Pinned 209dffe libTrophy.cpp:23,528..543 supplies all canonical identities.
// Only five source roles have independently observed full-prefixed scopes;
// neither mixed-prefix identities nor prefixed query/callback rows are granted.
const TROPHY2_REGISTRATIONS: [Registration; 20] = {
    use Service::*;
    let exports = [
        (
            Trophy2CreateContext,
            "Bagshr7OQ6Q",
            "LibNpTrophy2::NpTrophy2CreateContext",
            4,
        ),
        (
            Trophy2CreateHandle,
            "Gz1rmUZpROM",
            "LibNpTrophy2::NpTrophy2CreateHandle",
            1,
        ),
        (
            Trophy2RegisterContext,
            "bIDov3wBu5Q",
            "LibNpTrophy2::NpTrophy2RegisterContext",
            3,
        ),
        (
            Trophy2GetGameInfo,
            "4IzqhhUQ3nk",
            "LibNpTrophy2::NpTrophy2GetGameInfo",
            4,
        ),
        (
            Trophy2GetGroupInfo,
            "DoZWauG8mu0",
            "LibNpTrophy2::NpTrophy2GetGroupInfo",
            5,
        ),
        (
            Trophy2GetGroupInfoArray,
            "+PDSI6WgPRc",
            "LibNpTrophy2::NpTrophy2GetGroupInfoArray",
            7,
        ),
        (
            Trophy2GetTrophyInfo,
            "EwNylPdWUTM",
            "LibNpTrophy2::NpTrophy2GetTrophyInfo",
            5,
        ),
        (
            Trophy2GetTrophyInfoArray,
            "y3zHpdZO6ME",
            "LibNpTrophy2::NpTrophy2GetTrophyInfoArray",
            7,
        ),
        (
            Trophy2GetGameIcon,
            "2QgUy+xJqS0",
            "LibNpTrophy2::NpTrophy2GetGameIcon",
            4,
        ),
        (
            Trophy2GetGroupIcon,
            "6IjXJUy6ZnA",
            "LibNpTrophy2::NpTrophy2GetGroupIcon",
            5,
        ),
        (
            Trophy2GetTrophyIcon,
            "-9LLVU0uvs8",
            "LibNpTrophy2::NpTrophy2GetTrophyIcon",
            5,
        ),
        (
            Trophy2RegisterUnlockCallback,
            "sUXGfNMalIo",
            "LibNpTrophy2::NpTrophy2RegisterUnlockCallback",
            2,
        ),
        (
            Trophy2AbortHandle,
            "fYapWA9xVmA",
            "LibNpTrophy2::NpTrophy2AbortHandle",
            1,
        ),
        (
            Trophy2DestroyHandle,
            "d8P11CI40KE",
            "LibNpTrophy2::NpTrophy2DestroyHandle",
            1,
        ),
        (
            Trophy2DestroyContext,
            "sysY2FHYff4",
            "LibNpTrophy2::NpTrophy2DestroyContext",
            1,
        ),
    ];
    let mut entries = [BASE_REGISTRATIONS[0]; 20];
    let mut index = 0;
    while index < exports.len() {
        let (service, nid, source_name, arguments) = exports[index];
        entries[index] = Registration {
            service,
            nid,
            source_name,
            arguments,
            module: "NpTrophy2",
            library: "NpTrophy2",
            ..BASE_REGISTRATIONS[0]
        };
        index += 1;
    }
    let observed = [0, 1, 2, 14, 13];
    index = 0;
    while index < observed.len() {
        entries[15 + index] = Registration {
            module: "libSceNpTrophy2",
            library: "libSceNpTrophy2",
            ..entries[observed[index]]
        };
        index += 1;
    }
    entries
};

#[cfg(test)]
#[path = "catalog/trophy2.rs"]
mod trophy2_tests;

// Pinned 209dffe libTrophy.cpp:548..1323: all41 canonical definitions plus
// only20 independently observed full-prefix declarations, in source order.
// Float setters keep C semantic arity; their FP value is source logging-only
// and is never reinterpreted from an unused integer register.
const UDS_REGISTRATIONS: [Registration; 61] = {
    use Service::*;
    let exports = [
        (
            UdsInitialize,
            "sjaobBgqeB4",
            "LibNpUniversalDataSystem::NpUniversalDataSystemInitialize",
            1,
        ),
        (
            UdsCreateContext,
            "5zBnau1uIEo",
            "LibNpUniversalDataSystem::NpUniversalDataSystemCreateContext",
            4,
        ),
        (
            UdsCreateHandle,
            "hT0IAEvN+M0",
            "LibNpUniversalDataSystem::NpUniversalDataSystemCreateHandle",
            1,
        ),
        (
            UdsCreateEvent,
            "p+GcLqwpL9M",
            "LibNpUniversalDataSystem::NpUniversalDataSystemCreateEvent",
            4,
        ),
        (
            UdsPostEvent,
            "CzkKf7ahIyU",
            "LibNpUniversalDataSystem::NpUniversalDataSystemPostEvent",
            4,
        ),
        (
            UdsDestroyHandle,
            "AUIHb7jUX3I",
            "LibNpUniversalDataSystem::NpUniversalDataSystemDestroyHandle",
            1,
        ),
        (
            UdsAbortHandle,
            "jZCqWFgMehE",
            "LibNpUniversalDataSystem::NpUniversalDataSystemAbortHandle",
            1,
        ),
        (
            UdsDestroyContext,
            "wB7IWzGp2v0",
            "LibNpUniversalDataSystem::NpUniversalDataSystemDestroyContext",
            1,
        ),
        (
            UdsGetMemoryStat,
            "su7jW3VDDb4",
            "LibNpUniversalDataSystem::NpUniversalDataSystemGetMemoryStat",
            1,
        ),
        (
            UdsEventEstimateSize,
            "+s14jq-KGYw",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventEstimateSize",
            2,
        ),
        (
            UdsEventToString,
            "vj6CQGWtEBg",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventToString",
            4,
        ),
        (
            UdsDestroyEvent,
            "wG+84pnNIuo",
            "LibNpUniversalDataSystem::NpUniversalDataSystemDestroyEvent",
            1,
        ),
        (
            UdsRegisterContext,
            "tpFJ8LIKvPw",
            "LibNpUniversalDataSystem::NpUniversalDataSystemRegisterContext",
            3,
        ),
        (
            UdsCreateEventPropertyObject,
            "s6W4Zl4Slgk",
            "LibNpUniversalDataSystem::NpUniversalDataSystemCreateEventPropertyObject",
            1,
        ),
        (
            UdsDestroyEventPropertyObject,
            "kKUH0Viib3c",
            "LibNpUniversalDataSystem::NpUniversalDataSystemDestroyEventPropertyObject",
            1,
        ),
        (
            UdsEventPropertyObjectSetString,
            "MfDb+4Nln64",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyObjectSetString",
            3,
        ),
        (
            UdsEventPropertyObjectSetInt32,
            "YE4dbtbz6OE",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyObjectSetInt32",
            3,
        ),
        (
            UdsEventPropertyObjectSetUInt32,
            "AzD4irAcKE4",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyObjectSetUInt32",
            3,
        ),
        (
            UdsEventPropertyObjectSetInt64,
            "56QLTqx911s",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyObjectSetInt64",
            3,
        ),
        (
            UdsEventPropertyObjectSetUInt64,
            "xvsP5Yz6FmY",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyObjectSetUInt64",
            3,
        ),
        (
            UdsEventPropertyObjectSetFloat32,
            "lbPlT4+QVcE",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyObjectSetFloat32",
            3,
        ),
        (
            UdsEventPropertyObjectSetFloat64,
            "4Fu8tHW+u-k",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyObjectSetFloat64",
            3,
        ),
        (
            UdsEventPropertyObjectSetBool,
            "Fidd8vWgyVE",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyObjectSetBool",
            3,
        ),
        (
            UdsEventPropertyObjectSetBinary,
            "wAcxBDLHj1M",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyObjectSetBinary",
            4,
        ),
        (
            UdsEventPropertyObjectSetObject,
            "74ASEqxSnkM",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyObjectSetObject",
            4,
        ),
        (
            UdsEventPropertyObjectSetArray,
            "Wxbg5x3pTXA",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyObjectSetArray",
            4,
        ),
        (
            UdsCreateEventPropertyArray,
            "Hm7qubT3b70",
            "LibNpUniversalDataSystem::NpUniversalDataSystemCreateEventPropertyArray",
            1,
        ),
        (
            UdsDestroyEventPropertyArray,
            "W-0xwY0ZMjw",
            "LibNpUniversalDataSystem::NpUniversalDataSystemDestroyEventPropertyArray",
            1,
        ),
        (
            UdsEventPropertyArraySetString,
            "4llLk7YJRTE",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyArraySetString",
            2,
        ),
        (
            UdsEventPropertyArraySetInt32,
            "BypQuF113-k",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyArraySetInt32",
            2,
        ),
        (
            UdsEventPropertyArraySetUInt32,
            "yMi0xAOpmXM",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyArraySetUInt32",
            2,
        ),
        (
            UdsEventPropertyArraySetInt64,
            "viVXAwmmYrY",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyArraySetInt64",
            2,
        ),
        (
            UdsEventPropertyArraySetUInt64,
            "Qo9qR7v5zO4",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyArraySetUInt64",
            2,
        ),
        (
            UdsEventPropertyArraySetFloat32,
            "JmgwKm96Lq4",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyArraySetFloat32",
            2,
        ),
        (
            UdsEventPropertyArraySetFloat64,
            "sbSYZLR5AiE",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyArraySetFloat64",
            2,
        ),
        (
            UdsEventPropertyArraySetBool,
            "0+l4QSWCM4E",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyArraySetBool",
            2,
        ),
        (
            UdsEventPropertyArraySetBinary,
            "IEdUCV9j2Cw",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyArraySetBinary",
            3,
        ),
        (
            UdsEventPropertyArraySetObject,
            "XY14n3jNIpE",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyArraySetObject",
            3,
        ),
        (
            UdsEventPropertyArraySetArray,
            "rdi9BAfDLq8",
            "LibNpUniversalDataSystem::NpUniversalDataSystemEventPropertyArraySetArray",
            3,
        ),
        (
            UdsGetStorageStat,
            "KmN62tT4U8A",
            "LibNpUniversalDataSystem::NpUniversalDataSystemGetStorageStat",
            2,
        ),
        (
            UdsTerminate,
            "47UAEuQl+iI",
            "LibNpUniversalDataSystem::NpUniversalDataSystemTerminate",
            0,
        ),
    ];
    let mut entries = [BASE_REGISTRATIONS[0]; 61];
    let mut index = 0;
    while index < exports.len() {
        let (service, nid, source_name, arguments) = exports[index];
        entries[index] = Registration {
            service,
            nid,
            source_name,
            arguments,
            module: "NpUniversalDataSystem",
            library: "NpUniversalDataSystem",
            ..BASE_REGISTRATIONS[0]
        };
        index += 1;
    }
    let observed = [
        0, 1, 2, 3, 4, 5, 7, 11, 12, 13, 15, 16, 17, 18, 19, 22, 25, 28, 37, 40,
    ];
    index = 0;
    while index < observed.len() {
        entries[41 + index] = Registration {
            module: "libSceNpUniversalDataSystem",
            library: "libSceNpUniversalDataSystem",
            ..entries[observed[index]]
        };
        index += 1;
    }
    entries
};

#[cfg(test)]
#[path = "catalog/uds.rs"]
mod uds_tests;

const NETWORK_REGISTRATIONS: [Registration; 52] = {
    use Service::*;
    let exports = [
        (
            NetInit,
            "Nlev7Lg8k3A",
            "LibNet::NetInit",
            0,
            "Net",
            "libSceNet",
        ),
        (
            NetGetErrorAddr,
            "HQOwnfMGipQ",
            "LibNet::GetNetErrorAddr",
            0,
            "Net",
            "libSceNet",
        ),
        (
            NetPoolCreate,
            "dgJBaeJnGpo",
            "LibNet::NetPoolCreate",
            3,
            "Net",
            "libSceNet",
        ),
        (
            NetPoolDestroy,
            "K7RlrTkI-mw",
            "LibNet::NetPoolDestroy",
            1,
            "Net",
            "libSceNet",
        ),
        (
            NetResolverCreate,
            "C4UgDHHPvdw",
            "LibNet::NetResolverCreate",
            3,
            "Net",
            "libSceNet",
        ),
        (
            NetResolverDestroy,
            "kJlYH5uMAWI",
            "LibNet::NetResolverDestroy",
            1,
            "Net",
            "libSceNet",
        ),
        (
            NetResolverAbort,
            "AzqoBha7js4",
            "LibNet::NetResolverAbort",
            2,
            "Net",
            "libSceNet",
        ),
        (
            SslInit,
            "hdpVEUDFW3s",
            "Ssl::SslInit",
            1,
            "Ssl",
            "libSceSsl",
        ),
        (
            SslTerm,
            "0K1yQ6Lv-Yc",
            "Ssl::SslTerm",
            1,
            "Ssl",
            "libSceSsl",
        ),
        (
            SslGetCaCerts,
            "TDfQqO-gMbY",
            "Ssl::SslGetCaCerts",
            2,
            "Ssl",
            "libSceSsl",
        ),
        (
            SslFreeCaCerts,
            "qIvLs0gYxi0",
            "Ssl::SslFreeCaCerts",
            2,
            "Ssl",
            "libSceSsl",
        ),
        (
            HttpInit,
            "A9cVMUtEp4Y",
            "Http::HttpInit",
            3,
            "Http",
            "libSceHttp",
        ),
        (
            HttpTerm,
            "Ik-KpLTlf7Q",
            "Http::HttpTerm",
            1,
            "Http",
            "libSceHttp",
        ),
    ];
    let mut entries = [BASE_REGISTRATIONS[0]; 52];
    let mut index = 0;
    while index < exports.len() {
        let (service, nid, source_name, arguments, scope, prefixed) = exports[index];
        let entry = Registration {
            service,
            nid,
            source_name,
            arguments,
            module: scope,
            library: scope,
            ..BASE_REGISTRATIONS[0]
        };
        entries[index * 4] = entry;
        entries[index * 4 + 1] = Registration {
            module: prefixed,
            ..entry
        };
        entries[index * 4 + 2] = Registration {
            library: prefixed,
            ..entry
        };
        entries[index * 4 + 3] = Registration {
            module: prefixed,
            library: prefixed,
            ..entry
        };
        index += 1;
    }
    entries
};

const RUDP_REGISTRATIONS: [Registration; 12] = {
    let exports = [
        (Service::RudpInit, "amuBfI-AQc4", "Rudp::RudpInit"),
        (
            Service::RudpEnableInternalIOThread,
            "6PBNpsgyaxw",
            "Rudp::RudpEnableInternalIOThread",
        ),
        (
            Service::RudpSetEventHandler,
            "SUEVes8gvmw",
            "Rudp::RudpSetEventHandler",
        ),
    ];
    let mut entries = [BASE_REGISTRATIONS[0]; 12];
    let mut index = 0;
    while index < exports.len() {
        let (service, nid, source_name) = exports[index];
        let entry = Registration {
            service,
            nid,
            source_name,
            arguments: 2,
            module: "Rudp",
            library: "Rudp",
            ..BASE_REGISTRATIONS[0]
        };
        entries[index * 4] = entry;
        entries[index * 4 + 1] = Registration {
            module: "libSceRudp",
            ..entry
        };
        entries[index * 4 + 2] = Registration {
            library: "libSceRudp",
            ..entry
        };
        entries[index * 4 + 3] = Registration {
            module: "libSceRudp",
            library: "libSceRudp",
            ..entry
        };
        index += 1;
    }
    entries
};

const CONTENT_PARAMETER_REGISTRATIONS: [Registration; 4] = {
    let mut entries = [Registration {
        service: Service::AppContentAppParamGetInt,
        nid: "99b82IKXpH4",
        source_name: "AppContent::AppContentAppParamGetInt",
        arguments: 2,
        ..CONTENT_EVENT_REGISTRATIONS[0]
    }; 4];
    entries[1].module = "libSceAppContentUtil";
    entries[2].library = "libSceAppContent";
    entries[3].module = "libSceAppContentUtil";
    entries[3].library = "libSceAppContent";
    entries
};

const NET_CTL_REGISTRATIONS: [Registration; 40] = {
    let exports = [
        (Service::NetCtlInit, "gky0+oaNM4k", "NetCtl::NetCtlInit", 0),
        (Service::NetCtlTerm, "Z4wwCFiBELQ", "NetCtl::NetCtlTerm", 0),
        (
            Service::NetCtlGetState,
            "uBPlr0lbuiI",
            "NetCtl::NetCtlGetState",
            1,
        ),
        (
            Service::NetCtlGetStateV6,
            "+lxqIKeU9UY",
            "NetCtl::NetCtlGetStateV6",
            1,
        ),
        (
            Service::NetCtlGetNatInfo,
            "JO4yuTuMoKI",
            "NetCtl::NetCtlGetNatInfo",
            1,
        ),
        (
            Service::NetCtlRegisterCallback,
            "UJ+Z7Q+4ck0",
            "NetCtl::NetCtlRegisterCallback",
            3,
        ),
        (
            Service::NetCtlUnregisterCallback,
            "Rqm2OnZMCz0",
            "NetCtl::NetCtlUnregisterCallback",
            1,
        ),
        (
            Service::NetCtlCheckCallback,
            "iQw3iQPhvUQ",
            "NetCtl::NetCtlCheckCallback",
            0,
        ),
        (
            Service::NetCtlGetResult,
            "0cBgduPRR+M",
            "NetCtl::NetCtlGetResult",
            2,
        ),
        (
            Service::NetCtlGetInfo,
            "obuxdTiwkF8",
            "NetCtl::NetCtlGetInfo",
            2,
        ),
    ];
    let mut entries = [BASE_REGISTRATIONS[0]; 40];
    let mut index = 0;
    while index < exports.len() {
        let (service, nid, source_name, arguments) = exports[index];
        let entry = Registration {
            service,
            nid,
            source_name,
            arguments,
            module: "NetCtl",
            library: "NetCtl",
            ..BASE_REGISTRATIONS[0]
        };
        entries[index * 4] = entry;
        entries[index * 4 + 1] = Registration {
            module: "libSceNetCtl",
            ..entry
        };
        entries[index * 4 + 2] = Registration {
            library: "libSceNetCtl",
            ..entry
        };
        entries[index * 4 + 3] = Registration {
            module: "libSceNetCtl",
            library: "libSceNetCtl",
            ..entry
        };
        index += 1;
    }
    entries
};

const APR_SUBMIT_REGISTRATIONS: [Registration; 4] = [
    Registration {
        service: Service::AprSubmitCommandBufferAndGetResult,
        nid: "ASoW5WE-UPo",
        source_name: "Apr::SubmitCommandBufferAndGetResult",
        arguments: 4,
        ..BASE_REGISTRATIONS[0]
    },
    Registration {
        service: Service::AprSubmitCommandBuffer,
        nid: "eE4Szl8sil8",
        source_name: "Apr::SubmitCommandBuffer",
        arguments: 2,
        ..BASE_REGISTRATIONS[0]
    },
    Registration {
        service: Service::AprSubmitCommandBufferAndGetId,
        nid: "qvMUCyyaCSI",
        source_name: "Apr::SubmitCommandBufferAndGetId",
        arguments: 3,
        ..BASE_REGISTRATIONS[0]
    },
    Registration {
        service: Service::AprWaitCommandBuffer,
        nid: "rqwFKI4PAiM",
        source_name: "Apr::WaitCommandBuffer",
        arguments: 1,
        ..BASE_REGISTRATIONS[0]
    },
];

// Append new exact exports rather than shifting any earlier service marker.
const AMPR_READ_REGISTRATIONS: [Registration; 8] = {
    let base = [
        Registration {
            service: Service::AmprReadFile,
            nid: "mQ16-QdKv7k",
            source_name: "Ampr::AprCommandBufferReadFile",
            arguments: 7,
            ..AMPR_REGISTRATIONS[0]
        },
        Registration {
            service: Service::AmprMeasureReadFile,
            nid: "vWU-odnS+fU",
            source_name: "Ampr::MeasureCommandSizeReadFile",
            arguments: 4,
            ..AMPR_REGISTRATIONS[0]
        },
    ];
    let mut entries = [base[0]; 8];
    let mut index = 0;
    while index < base.len() {
        let entry = base[index];
        entries[index * 4] = entry;
        entries[index * 4 + 1] = Registration {
            module: "libSceAmpr",
            ..entry
        };
        entries[index * 4 + 2] = Registration {
            library: "libSceAmpr",
            ..entry
        };
        entries[index * 4 + 3] = Registration {
            module: "libSceAmpr",
            library: "libSceAmpr",
            ..entry
        };
        index += 1;
    }
    entries
};

const fn event_queue(
    service: Service,
    nid: &'static str,
    source_name: &'static str,
    arguments: u8,
) -> Registration {
    Registration {
        service,
        nid,
        source_name,
        arguments,
        ..BASE_REGISTRATIONS[0]
    }
}
const EVENT_QUEUE_REGISTRATIONS: [Registration; 16] = [
    event_queue(
        Service::CreateEqueue,
        "D0OdFMjp46I",
        "KernelCreateEqueue",
        2,
    ),
    event_queue(
        Service::DeleteEqueue,
        "jpFjmgAC5AE",
        "KernelDeleteEqueue",
        1,
    ),
    event_queue(Service::WaitEqueue, "fzyMKs9kim0", "KernelWaitEqueue", 5),
    event_queue(
        Service::GetEventUserData,
        "vz+pg2zdopI",
        "KernelGetEventUserData",
        1,
    ),
    event_queue(Service::GetEventId, "mJ7aghmgvfc", "KernelGetEventId", 1),
    event_queue(
        Service::GetEventFilter,
        "23CPPI1tyBY",
        "KernelGetEventFilter",
        1,
    ),
    event_queue(
        Service::GetEventData,
        "kwGyyjohI50",
        "KernelGetEventData",
        1,
    ),
    event_queue(
        Service::GetEventFflags,
        "Q0qr9AyqJSk",
        "KernelGetEventFflags",
        1,
    ),
    event_queue(
        Service::GetEventError,
        "Uu-iDFC9aUc",
        "KernelGetEventError",
        1,
    ),
    event_queue(
        Service::AddUserEvent,
        "4R6-OvI2cEA",
        "KernelAddUserEvent",
        2,
    ),
    event_queue(
        Service::AddUserEventEdge,
        "WDszmSbWuDk",
        "KernelAddUserEventEdge",
        2,
    ),
    event_queue(
        Service::TriggerUserEvent,
        "F6e0kwo4cnk",
        "KernelTriggerUserEvent",
        3,
    ),
    event_queue(
        Service::DeleteUserEvent,
        "LJDwdSNTnDg",
        "KernelDeleteUserEvent",
        2,
    ),
    event_queue(
        Service::AddTimerEvent,
        "57ZK+ODEXWY",
        "KernelAddTimerEvent",
        4,
    ),
    event_queue(
        Service::AddHRTimerEvent,
        "R74tt43xP6k",
        "KernelAddHRTimerEvent",
        4,
    ),
    event_queue(
        Service::DeleteHRTimerEvent,
        "J+LF6LwObXU",
        "KernelDeleteHRTimerEvent",
        2,
    ),
];

const HARDWARE_REGISTRATIONS: [Registration; 2] = [
    Registration {
        service: Service::IsTrinityMode,
        nid: "tU5e3f9gSiU",
        source_name: "KernelIsTrinityMode",
        arguments: 0,
        ..BASE_REGISTRATIONS[0]
    },
    Registration {
        service: Service::GetOperationMode,
        nid: "NH6xARDOVv8",
        source_name: "KernelGetOperationMode",
        arguments: 2,
        ..BASE_REGISTRATIONS[0]
    },
];

const PAD_REGISTRATIONS: [Registration; 20] = {
    use Service::*;
    let base = [
        (PadInit, "hv1luiJrqQM", "PadInit", 0),
        (PadOpen, "xk0AcarP3V4", "PadOpen", 4),
        (PadGetHandle, "u1GRHp+oWoY", "PadGetHandle", 3),
        (PadClose, "6ncge5+l5Qs", "PadClose", 1),
        (PadIsRemote, "fCWdlnmB1Ks", "PadIsRemoteController", 2),
    ];
    let mut entries = [BASE_REGISTRATIONS[0]; 20];
    let mut index = 0;
    while index < base.len() {
        let (service, nid, source_name, arguments) = base[index];
        let entry = Registration {
            service,
            nid,
            source_name,
            arguments,
            module: "Pad",
            library: "Pad",
            ..BASE_REGISTRATIONS[0]
        };
        entries[index * 4] = entry;
        entries[index * 4 + 1] = Registration {
            module: "libScePad",
            ..entry
        };
        entries[index * 4 + 2] = Registration {
            library: "libScePad",
            ..entry
        };
        entries[index * 4 + 3] = Registration {
            module: "libScePad",
            library: "libScePad",
            ..entry
        };
        index += 1;
    }
    entries
};

const AMPR_REGISTRATIONS: [Registration; 48] = {
    use Service::*;
    let base = [
        (
            AmprConstruct,
            "8aI7R7WaOlc",
            "AmprCommandBufferConstructor",
            1,
        ),
        (
            AmprAprConstruct,
            "a8uLzYY--tM",
            "AmprAprCommandBufferConstructor",
            3,
        ),
        (
            AmprAprDestruct,
            "Qs1xtplKo0U",
            "AmprAprCommandBufferDestructor",
            1,
        ),
        (
            AmprDestruct,
            "GuchCTefuZw",
            "AmprCommandBufferDestructor",
            1,
        ),
        (
            AmprSetBuffer,
            "N-FSPA4S3nI",
            "AmprCommandBufferSetBuffer",
            3,
        ),
        (AmprReset, "baQO9ez2gL4", "AmprCommandBufferReset", 1),
        (
            AmprClearBuffer,
            "ULvXMDz56po",
            "AmprCommandBufferClearBuffer",
            1,
        ),
        (AmprGetType, "VEDMaQmJZng", "AmprCommandBufferGetType", 1),
        (
            AmprGetOffset,
            "GnxKOHEawhk",
            "AmprCommandBufferGetCurrentOffset",
            1,
        ),
        (
            AmprGetNum,
            "gzndltBEzWc",
            "AmprCommandBufferGetNumCommands",
            1,
        ),
        (AmprGetSize, "tZDDEo2tE5k", "AmprCommandBufferGetSize", 1),
        (
            AmprGetBase,
            "RPCAhx-aabE",
            "AmprCommandBufferGetBufferBaseAddress",
            1,
        ),
    ];
    let mut entries = [BASE_REGISTRATIONS[0]; 48];
    let mut index = 0;
    while index < base.len() {
        let (service, nid, source_name, arguments) = base[index];
        let entry = Registration {
            service,
            nid,
            source_name,
            arguments,
            module: "Ampr",
            library: "Ampr",
            ..BASE_REGISTRATIONS[0]
        };
        entries[index * 4] = entry;
        entries[index * 4 + 1] = Registration {
            module: "libSceAmpr",
            ..entry
        };
        entries[index * 4 + 2] = Registration {
            library: "libSceAmpr",
            ..entry
        };
        entries[index * 4 + 3] = Registration {
            module: "libSceAmpr",
            library: "libSceAmpr",
            ..entry
        };
        index += 1;
    }
    entries
};

const SAVE_REGISTRATIONS: [Registration; 88] = {
    use Service::*;
    let base = [
        (SaveInitialize, "TywrFKCoLGY", "SaveDataInitialize3", 1),
        (SaveTerminate, "yKDy8S5yLA0", "SaveDataTerminate", 0),
        (
            SaveCreateTransaction,
            "gjRZNnw0JPE",
            "SaveDataCreateTransactionResource",
            1,
        ),
        (
            SaveDeleteTransaction,
            "lJUQuaKqoKY",
            "SaveDataDeleteTransactionResource",
            1,
        ),
        (SavePrepare, "sDCBrmc61XU", "SaveDataPrepare", 2),
        (SaveCommit, "ie7qhZ4X0Cc", "SaveDataCommit", 1),
        (SaveGetEvent, "j8xKtiFj0SY", "SaveDataGetEventResult", 2),
        (
            SaveMemorySetup,
            "oQySEUfgXRA",
            "SaveDataSetupSaveDataMemory2",
            2,
        ),
        (
            SaveMemoryGet,
            "QwOO7vegnV8",
            "SaveDataGetSaveDataMemory2",
            1,
        ),
        (
            SaveMemorySet,
            "cduy9v4YmT4",
            "SaveDataSetSaveDataMemory2",
            1,
        ),
        (
            SaveMemorySync,
            "wiT9jeC7xPw",
            "SaveDataSyncSaveDataMemory",
            1,
        ),
    ];
    let mut entries = [BASE_REGISTRATIONS[0]; 88];
    let mut index = 0;
    while index < base.len() {
        let (service, nid, source_name, arguments) = base[index];
        let entry = Registration {
            service,
            nid,
            source_name,
            arguments,
            module: "SaveData",
            library: "SaveData",
            ..BASE_REGISTRATIONS[0]
        };
        entries[index * 8] = entry;
        entries[index * 8 + 1] = Registration {
            module: "libSceSaveData",
            ..entry
        };
        entries[index * 8 + 2] = Registration {
            library: "libSceSaveData",
            ..entry
        };
        entries[index * 8 + 3] = Registration {
            module: "libSceSaveData",
            library: "libSceSaveData",
            ..entry
        };
        entries[index * 8 + 4] = Registration {
            module: "SaveData_native",
            library: "SaveData_native",
            ..entry
        };
        entries[index * 8 + 5] = Registration {
            module: "libSceSaveData_native",
            library: "SaveData_native",
            ..entry
        };
        entries[index * 8 + 6] = Registration {
            module: "SaveData_native",
            library: "libSceSaveData_native",
            ..entry
        };
        entries[index * 8 + 7] = Registration {
            module: "libSceSaveData_native",
            library: "libSceSaveData_native",
            ..entry
        };
        index += 1;
    }
    entries
};

const NP_REGISTRATIONS: [Registration; 68] = {
    use Service::*;
    let base = [
        (NpGetId, "p-o74CnoNzY", "NpGetNpId", 2),
        (NpGetOnlineId, "XDncXQIJUSk", "NpGetOnlineId", 2),
        (NpGetAccountId, "rbknaUjpqWo", "NpGetAccountIdA", 2),
        (
            NpGetAccountCountry,
            "JT+t00a3TxA",
            "NpGetAccountCountryA",
            2,
        ),
        (NpGetAccountAge, "+4DegjBqV1g", "NpGetAccountAge", 3),
        (
            NpGetAccountLanguage,
            "3Tcz5bNCfZQ",
            "NpGetAccountLanguage2",
            3,
        ),
        (NpGetState, "eQH7nWPcAgc", "NpGetState", 2),
        (
            NpGetReachabilityState,
            "e-ZuhGEoeC4",
            "NpGetNpReachabilityState",
            2,
        ),
        (NpHasSignedUp, "Oad3rvY-NJQ", "NpHasSignedUp", 2),
        (NpCreateRequest, "GpLQDNKICac", "NpCreateRequest", 0),
        (
            NpCreateAsyncRequest,
            "eiqMCt9UshI",
            "NpCreateAsyncRequest",
            1,
        ),
        (NpDeleteRequest, "S7QTn72PrDw", "NpDeleteRequest", 1),
        (NpAbortRequest, "OzKvTvg3ZYU", "NpAbortRequest", 1),
        (
            NpCheckAvailability,
            "2rsFmlGWleQ",
            "NpCheckNpAvailability",
            3,
        ),
        (
            NpCheckReachability,
            "KfGZg2y73oM",
            "NpCheckNpReachability",
            2,
        ),
        (NpPollAsync, "uqcPJLWL08M", "NpPollAsync", 2),
        (NpCheckPremium, "O80NrhUOPGY", "NpCheckPremium", 3),
    ];
    let mut entries = [BASE_REGISTRATIONS[0]; 68];
    let mut index = 0;
    while index < base.len() {
        let (service, nid, source_name, arguments) = base[index];
        let entry = Registration {
            service,
            nid,
            source_name,
            arguments,
            module: "NpManager",
            library: "NpManager",
            ..BASE_REGISTRATIONS[0]
        };
        entries[index * 4] = entry;
        entries[index * 4 + 1] = Registration {
            module: "libSceNpManager",
            ..entry
        };
        entries[index * 4 + 2] = Registration {
            library: "libSceNpManager",
            ..entry
        };
        entries[index * 4 + 3] = Registration {
            module: "libSceNpManager",
            library: "libSceNpManager",
            ..entry
        };
        index += 1;
    }
    entries
};

const USER_REGISTRATIONS: [Registration; 64] = {
    use Service::*;
    let base = [
        (UserInitialize, "j3YMu1MVNNo", "UserServiceInitialize", 1),
        (UserInitialize2, "az-0R6eviZ0", "UserServiceInitialize2", 0),
        (
            UserGetInitial,
            "CdWp0oHWGr0",
            "UserServiceGetInitialUser",
            1,
        ),
        (UserGetEvent, "yH17Q6NWtVg", "UserServiceGetEvent", 1),
        (
            UserGetLoginList,
            "fPhymKNvK-A",
            "UserServiceGetLoginUserIdList",
            1,
        ),
        (UserGetName, "1xxcMiGu2fo", "UserServiceGetUserName", 3),
        (UserGetNumber, "bwFjS+bX9mA", "UserServiceGetUserNumber", 2),
        (UserGetNumber, "qbwy0Ub8b3M", "UserServiceGetUserNumber", 2),
        (
            UserGetGamePresets,
            "-sD02mFDBh4",
            "UserServiceGetGamePresets",
            2,
        ),
        (UserGetAgeLevel, "woNpu+45RLk", "UserServiceGetAgeLevel", 2),
        (
            UserGetChatTranscription,
            "rnEhHqG-4xo",
            "UserServiceGetAccessibilityChatTranscription",
            2,
        ),
        (
            UserGetPressAndHoldDelay,
            "ZKJtxdgvzwg",
            "UserServiceGetAccessibilityPressAndHoldDelay",
            2,
        ),
        (
            UserGetVibration,
            "qWYHOFwqCxY",
            "UserServiceGetAccessibilityVibration",
            2,
        ),
        (
            UserGetTriggerEffect,
            "-3Y5GO+-i78",
            "UserServiceGetAccessibilityTriggerEffect",
            2,
        ),
        (
            UserGetZoomEnabled,
            "hD-H81EN9Vg",
            "UserServiceGetAccessibilityZoomEnabled",
            2,
        ),
        (
            UserGetZoomFollowFocus,
            "O6IW1-Dwm-w",
            "UserServiceGetAccessibilityZoomFollowFocus",
            2,
        ),
    ];
    let mut entries = [BASE_REGISTRATIONS[0]; 64];
    let mut index = 0;
    while index < base.len() {
        let (service, nid, source_name, arguments) = base[index];
        let entry = Registration {
            service,
            nid,
            source_name,
            arguments,
            module: "UserService",
            library: "UserService",
            ..BASE_REGISTRATIONS[0]
        };
        entries[index * 4] = entry;
        entries[index * 4 + 1] = Registration {
            module: "libSceUserService",
            ..entry
        };
        entries[index * 4 + 2] = Registration {
            library: "libSceUserService",
            ..entry
        };
        entries[index * 4 + 3] = Registration {
            module: "libSceUserService",
            library: "libSceUserService",
            ..entry
        };
        index += 1;
    }
    entries
};

const CONTENT_EVENT_REGISTRATIONS: [Registration; 12] = {
    let base = [
        Registration {
            service: Service::AppContentInitialize,
            nid: "R9lA82OraNs",
            source_name: "AppContentInitialize",
            arguments: 2,
            module: "AppContentUtil",
            library: "AppContent",
            ..BASE_REGISTRATIONS[0]
        },
        Registration {
            service: Service::SystemReceiveEvent,
            nid: "656LMQSrg6U",
            source_name: "SystemServiceReceiveEvent",
            arguments: 1,
            module: "SystemService",
            library: "SystemService",
            ..BASE_REGISTRATIONS[0]
        },
        Registration {
            service: Service::SystemGetStatus,
            nid: "rPo6tV8D9bM",
            source_name: "SystemServiceGetStatus",
            arguments: 1,
            module: "SystemService",
            library: "SystemService",
            ..BASE_REGISTRATIONS[0]
        },
    ];
    let mut entries = [base[0]; 12];
    let mut index = 0;
    while index < base.len() {
        let entry = base[index];
        let (module, library) = if index == 0 {
            ("libSceAppContentUtil", "libSceAppContent")
        } else {
            ("libSceSystemService", "libSceSystemService")
        };
        entries[index * 4] = entry;
        entries[index * 4 + 1] = Registration { module, ..entry };
        entries[index * 4 + 2] = Registration { library, ..entry };
        entries[index * 4 + 3] = Registration {
            module,
            library,
            ..entry
        };
        index += 1;
    }
    entries
};

// Append after existing entries so previous executable marker IDs stay stable.
const DIALOG_REGISTRATIONS: [Registration; 8] = {
    let first = Registration {
        service: Service::CommonDialogInitialize,
        nid: "uoUpLGNkygk",
        source_name: "CommonDialogInitialize",
        arguments: 0,
        module: "CommonDialog",
        library: "CommonDialog",
        ..BASE_REGISTRATIONS[0]
    };
    let second = Registration {
        service: Service::CommonDialogIsUsed,
        nid: "BQ3tey0JmQM",
        source_name: "CommonDialogIsUsed",
        ..first
    };
    let base = [first, second];
    let mut entries = [first; 8];
    let mut index = 0;
    while index < base.len() {
        let entry = base[index];
        entries[index * 4] = entry;
        entries[index * 4 + 1] = Registration {
            module: "libSceCommonDialog",
            ..entry
        };
        entries[index * 4 + 2] = Registration {
            library: "libSceCommonDialog",
            ..entry
        };
        entries[index * 4 + 3] = Registration {
            module: "libSceCommonDialog",
            library: "libSceCommonDialog",
            ..entry
        };
        index += 1;
    }
    entries
};

const FIBER_BASE: [Registration; 6] = [
    fiber_registration(
        Service::FiberInitialize,
        "hVYD7Ou2pCQ",
        "FiberInitialize",
        8,
    ),
    fiber_registration(
        Service::FiberInitializeInternal,
        "7+OJIpko9RY",
        "FiberInitializeInternal",
        9,
    ),
    fiber_registration(
        Service::FiberOptParamInitialize,
        "asjUJJ+aa8s",
        "FiberOptParamInitialize",
        1,
    ),
    fiber_registration(Service::FiberFinalize, "JeNX5F-NzQU", "FiberFinalize", 1),
    fiber_registration(Service::FiberGetInfo, "uq2Y5BFz0PE", "FiberGetInfo", 2),
    fiber_registration(Service::FiberRename, "JzyT91ucGDc", "FiberRename", 2),
];
const fn fiber_registration(
    service: Service,
    nid: &'static str,
    source_name: &'static str,
    arguments: u8,
) -> Registration {
    Registration {
        service,
        nid,
        source_name,
        arguments,
        module: "Fiber",
        library: "Fiber",
        ..BASE_REGISTRATIONS[0]
    }
}
const FIBER_REGISTRATIONS: [Registration; 24] = {
    let mut entries = [FIBER_BASE[0]; 24];
    let mut index = 0;
    while index < FIBER_BASE.len() {
        let entry = FIBER_BASE[index];
        entries[index * 4] = entry;
        entries[index * 4 + 1] = Registration {
            module: "libSceFiber",
            ..entry
        };
        entries[index * 4 + 2] = Registration {
            library: "libSceFiber",
            ..entry
        };
        entries[index * 4 + 3] = Registration {
            module: "libSceFiber",
            library: "libSceFiber",
            ..entry
        };
        index += 1;
    }
    entries
};

const OUTPUT_REGISTRATIONS: &[Registration] = &[
    Registration {
        service: Service::Write,
        nid: "4wSze92BhLI",
        source_name: "KernelWrite",
        arguments: 3,
        ..BASE_REGISTRATIONS[0]
    },
    Registration {
        service: Service::Write,
        nid: "FN4gaPmuFV8",
        source_name: "KernelWrite",
        arguments: 3,
        ..BASE_REGISTRATIONS[0]
    },
    Registration {
        service: Service::PosixWrite,
        nid: "FxVZqBAA7ks",
        source_name: "write",
        arguments: 3,
        ..BASE_REGISTRATIONS[0]
    },
];

const CALENDAR_REGISTRATIONS: [Registration; 3] = [
    Registration {
        service: Service::Gettimezone,
        nid: "kOcnerypnQA",
        source_name: "KernelGettimezone",
        arguments: 1,
        ..BASE_REGISTRATIONS[0]
    },
    Registration {
        service: Service::ConvertUtcToLocaltime,
        nid: "-o5uEDpN+oY",
        source_name: "KernelConvertUtcToLocaltime",
        arguments: 4,
        ..BASE_REGISTRATIONS[0]
    },
    Registration {
        service: Service::ConvertLocaltimeToUtc,
        nid: "0NTHN1NKONI",
        source_name: "KernelConvertLocaltimeToUtc",
        arguments: 5,
        ..BASE_REGISTRATIONS[0]
    },
];

// SymbolDatabase::UpdateName normalizes each namespace independently. Export
// only these four reviewed spellings, preserving exact version/type matching.
const SYSTEM_REGISTRATIONS: [Registration; 4] = {
    let entry = Registration {
        service: Service::SystemParamGetInt,
        nid: "fZo48un7LK4",
        source_name: "SystemServiceParamGetInt",
        arguments: 2,
        module: "SystemService",
        library: "SystemService",
        module_version: (1, 1),
        library_version: 1,
    };
    [
        entry,
        Registration {
            module: "libSceSystemService",
            ..entry
        },
        Registration {
            library: "libSceSystemService",
            ..entry
        },
        Registration {
            module: "libSceSystemService",
            library: "libSceSystemService",
            ..entry
        },
    ]
};

// Append these exports after all existing groups to preserve earlier marker IDs.
// Each namespace is normalized independently by the pinned symbol database.
const SYSTEM_DISPLAY_REGISTRATIONS: [Registration; 8] = {
    let base = [
        Registration {
            service: Service::SystemGetHdrToneMapLuminance,
            nid: "mPpPxv5CZt4",
            source_name: "SystemServiceGetHdrToneMapLuminance",
            arguments: 1,
            ..SYSTEM_REGISTRATIONS[0]
        },
        Registration {
            service: Service::SystemGetDisplaySafeAreaInfo,
            nid: "1n37q1Bvc5Y",
            source_name: "SystemServiceGetDisplaySafeAreaInfo",
            arguments: 1,
            ..SYSTEM_REGISTRATIONS[0]
        },
    ];
    let mut entries = [base[0]; 8];
    let mut index = 0;
    while index < base.len() {
        let entry = base[index];
        entries[index * 4] = entry;
        entries[index * 4 + 1] = Registration {
            module: "libSceSystemService",
            ..entry
        };
        entries[index * 4 + 2] = Registration {
            library: "libSceSystemService",
            ..entry
        };
        entries[index * 4 + 3] = Registration {
            module: "libSceSystemService",
            library: "libSceSystemService",
            ..entry
        };
        index += 1;
    }
    entries
};

const SYSMODULE_BASE: [Registration; 4] = {
    let entry = Registration {
        service: Service::SysmoduleLoad,
        nid: "g8cM39EUZ6o",
        source_name: "SysmoduleLoadModule",
        arguments: 1,
        module: "Sysmodule",
        library: "Sysmodule",
        module_version: (1, 1),
        library_version: 1,
    };
    [
        entry,
        Registration {
            service: Service::SysmoduleUnload,
            nid: "eR2bZFAAU0Q",
            source_name: "SysmoduleUnloadModule",
            ..entry
        },
        Registration {
            service: Service::SysmoduleIsLoaded,
            nid: "fMP5NHUOaMk",
            source_name: "SysmoduleIsLoaded",
            ..entry
        },
        Registration {
            service: Service::SysmoduleLoadInternal,
            nid: "hHrGoGoNf+s",
            source_name: "SysmoduleLoadModuleInternalWithArg",
            arguments: 5,
            ..entry
        },
    ]
};

// Same independent namespace normalization as the pinned SymbolDatabase.
// Exact versions/types still participate in ordinary linker resolution.
const SYSMODULE_REGISTRATIONS: [Registration; 16] = {
    let mut entries = [SYSMODULE_BASE[0]; 16];
    let mut index = 0;
    while index < SYSMODULE_BASE.len() {
        let entry = SYSMODULE_BASE[index];
        entries[index * 4] = entry;
        entries[index * 4 + 1] = Registration {
            module: "libSceSysmodule",
            ..entry
        };
        entries[index * 4 + 2] = Registration {
            library: "libSceSysmodule",
            ..entry
        };
        entries[index * 4 + 3] = Registration {
            module: "libSceSysmodule",
            library: "libSceSysmodule",
            ..entry
        };
        index += 1;
    }
    entries
};

const VIRTUAL_NAME_REGISTRATIONS: [Registration; 2] = [
    Registration {
        service: Service::SetVirtualRangeName,
        nid: "DGMG3JshrZU",
        source_name: "KernelSetVirtualRangeName",
        arguments: 3,
        ..BASE_REGISTRATIONS[0]
    },
    Registration {
        service: Service::ClearVirtualRangeName,
        nid: "mkgXxsoxWHg",
        source_name: "KernelClearVirtualRangeName",
        arguments: 2,
        ..BASE_REGISTRATIONS[0]
    },
];

const CONDITION_WAIT_REGISTRATIONS: [Registration; 3] = [
    Registration {
        service: Service::CondWait,
        nid: "WKAXJ4XBPQ4",
        source_name: "PthreadCondWait",
        arguments: 2,
        ..BASE_REGISTRATIONS[0]
    },
    Registration {
        service: Service::PosixCondWait,
        nid: "Op8TBGY5KHg",
        source_name: "pthread_cond_wait",
        arguments: 2,
        library: "Posix",
        ..BASE_REGISTRATIONS[0]
    },
    Registration {
        service: Service::PosixCondWait,
        nid: "Op8TBGY5KHg",
        source_name: "pthread_cond_wait",
        arguments: 2,
        library: "libScePosix",
        ..BASE_REGISTRATIONS[0]
    },
];

const fn semaphore_registration(
    service: Service,
    nid: &'static str,
    source_name: &'static str,
    arguments: u8,
    library: &'static str,
) -> Registration {
    Registration {
        service,
        nid,
        source_name,
        arguments,
        library,
        ..BASE_REGISTRATIONS[0]
    }
}

const SEMAPHORE_BASE: [Registration; 15] = [
    semaphore_registration(Service::PosixSemInit, "pDuPEf3m4fI", "sem_init", 3, "Posix"),
    semaphore_registration(
        Service::PosixSemDestroy,
        "cDW233RAwWo",
        "sem_destroy",
        1,
        "Posix",
    ),
    semaphore_registration(Service::PosixSemWait, "YCV5dGGBcCo", "sem_wait", 1, "Posix"),
    semaphore_registration(
        Service::PosixSemTryWait,
        "WBWzsRifCEA",
        "sem_trywait",
        1,
        "Posix",
    ),
    semaphore_registration(
        Service::PosixSemRelativeWait,
        "4SbrhCozqQU",
        "sem_reltimedwait_np",
        2,
        "Posix",
    ),
    semaphore_registration(
        Service::PosixSemTimedWait,
        "w5IHyvahg-o",
        "sem_timedwait",
        2,
        "Posix",
    ),
    semaphore_registration(Service::PosixSemPost, "IKP8typ0QUk", "sem_post", 1, "Posix"),
    semaphore_registration(
        Service::PosixSemGetValue,
        "Bq+LRV-N6Hk",
        "sem_getvalue",
        2,
        "Posix",
    ),
    semaphore_registration(
        Service::PthreadSemInit,
        "GEnUkDZoUwY",
        "PthreadSemInit",
        4,
        "libkernel",
    ),
    semaphore_registration(
        Service::PthreadSemDestroy,
        "Vwc+L05e6oE",
        "PthreadSemDestroy",
        1,
        "libkernel",
    ),
    semaphore_registration(
        Service::PthreadSemWait,
        "C36iRE0F5sE",
        "PthreadSemWait",
        1,
        "libkernel",
    ),
    semaphore_registration(
        Service::PthreadSemTryWait,
        "H2a+IN9TP0E",
        "PthreadSemTrywait",
        1,
        "libkernel",
    ),
    semaphore_registration(
        Service::PthreadSemTimedWait,
        "fjN6NQHhK8k",
        "PthreadSemTimedwait",
        2,
        "libkernel",
    ),
    semaphore_registration(
        Service::PthreadSemPost,
        "aishVAiFaYM",
        "PthreadSemPost",
        1,
        "libkernel",
    ),
    semaphore_registration(
        Service::PthreadSemGetValue,
        "DjpBvGlaWbQ",
        "PthreadSemGetvalue",
        2,
        "libkernel",
    ),
];

const SEMAPHORE_REGISTRATIONS: [Registration; 23] = {
    let mut entries = [SEMAPHORE_BASE[0]; 23];
    let mut index = 0;
    let mut output = 0;
    while index < SEMAPHORE_BASE.len() {
        let entry = SEMAPHORE_BASE[index];
        entries[output] = entry;
        output += 1;
        if is_posix(&entry) {
            entries[output] = Registration {
                library: "libScePosix",
                ..entry
            };
            output += 1;
        }
        index += 1;
    }
    assert!(output == entries.len());
    entries
};

#[must_use]
pub fn resolve(
    module: &str,
    module_version: (u16, u16),
    library: &str,
    library_version: u16,
    nid: &str,
) -> Option<Service> {
    REGISTRATIONS
        .iter()
        .find(|entry| {
            entry.module == module
                && entry.module_version == module_version
                && entry.library == library
                && entry.library_version == library_version
                && entry.nid == nid
        })
        .map(|entry| entry.service)
}

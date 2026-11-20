#import <Foundation/Foundation.h>
#import <NaturalLanguage/NaturalLanguage.h>
#import <ApplicationServices/ApplicationServices.h>
#import <IOKit/ps/IOPowerSources.h>
#import <IOKit/ps/IOPSKeys.h>
#import <UserNotifications/UserNotifications.h>
#include <stdlib.h>
#include <pthread/qos.h>
void sumi_background_thread(void) {
    pthread_set_qos_class_self_np(QOS_CLASS_BACKGROUND, 0);
}
int sumi_quiet_power(double seconds) {
 @autoreleasepool {
   if ([NSProcessInfo processInfo].thermalState >= NSProcessInfoThermalStateSerious) return 0;
   CFTypeRef info=IOPSCopyPowerSourcesInfo();
   if(!info)return 0;
   CFStringRef power=IOPSGetProvidingPowerSourceType(info);
   BOOL onAC=power&&CFEqual(power,CFSTR(kIOPSACPowerValue));
   CFRelease(info);
   return onAC && CGEventSourceSecondsSinceLastEventType(kCGEventSourceStateCombinedSessionState,kCGAnyInputEventType)>seconds;
 }
}
int sumi_heavy_allowed(void) {return sumi_quiet_power(90.);}
char *sumi_entities(const char *input) {
 @autoreleasepool {
  NSString *s=[NSString stringWithUTF8String:input];
  if(!s)return strdup("[]");
  NLTagger *tagger=[[NLTagger alloc]initWithTagSchemes:@[NLTagSchemeNameType]];
  tagger.string=s;NSMutableArray *names=[NSMutableArray array];
  [tagger enumerateTagsInRange:NSMakeRange(0,s.length) unit:NLTokenUnitWord scheme:NLTagSchemeNameType options:NLTaggerOmitWhitespace|NLTaggerJoinNames usingBlock:^(NLTag tag,NSRange range,BOOL *stop){
    (void)stop; if([tag isEqualToString:NLTagPersonalName]) [names addObject:[s substringWithRange:range]];
  }];
  NSData *data=[NSJSONSerialization dataWithJSONObject:names options:0 error:nil];
  return strdup([[[NSString alloc]initWithData:data encoding:NSUTF8StringEncoding] UTF8String]);
 }
}
void sumi_free(char *p){free(p);}
int sumi_semantic(const char *input, float *result, int length) {
    @autoreleasepool {
        static NLEmbedding *embedding;
        static dispatch_once_t once;
        dispatch_once(&once, ^{
            embedding = [NLEmbedding sentenceEmbeddingForLanguage:NLLanguageEnglish];
        });
        if (!embedding || length <= 0) return 0;
        NSString *text = [NSString stringWithUTF8String:input];
        // A notebook import and a Desk request can share the same native model.
        @synchronized (embedding) {
            NSArray<NSNumber *> *vector = [embedding vectorForString:text];
            if (!vector.count) return 0;
            for (NSUInteger i = 0; i < vector.count; i++) {
                result[i % length] += vector[i].floatValue;
            }
        }
        return 1;
    }
}

static dispatch_group_t sumi_notification_permission(void) {
    static dispatch_group_t group;
    static dispatch_once_t once;
    dispatch_once(&once, ^{ group = dispatch_group_create(); });
    return group;
}
void sumi_request_notifications(void) {
    @autoreleasepool {
        dispatch_group_t group = sumi_notification_permission();
        dispatch_group_enter(group);
        [[UNUserNotificationCenter currentNotificationCenter]
            requestAuthorizationWithOptions:UNAuthorizationOptionAlert
            completionHandler:^(BOOL granted, NSError *error) {
                (void)granted;
                (void)error;
                dispatch_group_leave(group);
            }];
    }
}
void sumi_notify_when_ready(void) {
    // A small notebook can finish before the person answers the permission sheet.
    dispatch_group_notify(sumi_notification_permission(), dispatch_get_main_queue(), ^{
        UNUserNotificationCenter *center = [UNUserNotificationCenter currentNotificationCenter];
        [center getNotificationSettingsWithCompletionHandler:^(UNNotificationSettings *settings) {
            if (settings.authorizationStatus != UNAuthorizationStatusAuthorized &&
                settings.authorizationStatus != UNAuthorizationStatusProvisional) return;
            UNMutableNotificationContent *content = [[UNMutableNotificationContent alloc] init];
            content.title = @"Sumi";
            content.body = @"Your first reading is ready.";
            UNNotificationRequest *request = [UNNotificationRequest
                requestWithIdentifier:@"first-reading" content:content trigger:nil];
            [center addNotificationRequest:request withCompletionHandler:nil];
        }];
    });
}

#import <PDFKit/PDFKit.h>
#import <Vision/Vision.h>
#import <ImageIO/ImageIO.h>
#import <AppKit/AppKit.h>

static NSString *readImage(CGImageRef image) {
    VNRecognizeTextRequest *text = [VNRecognizeTextRequest new];
    VNClassifyImageRequest *labels = [VNClassifyImageRequest new];
    VNImageRequestHandler *handler = [[VNImageRequestHandler alloc] initWithCGImage:image options:@{}];
    if (![handler performRequests:@[text,labels] error:nil]) return nil;
    NSMutableArray *lines = [NSMutableArray array];
    for (VNRecognizedTextObservation *row in text.results) {
        NSString *value = [row topCandidates:1].firstObject.string;
        if (value) [lines addObject:value];
    }
    NSMutableArray *objects=[NSMutableArray array];
    for (VNClassificationObservation *label in labels.results) {
        if (label.confidence >= 0.35 && objects.count < 5) [objects addObject:label.identifier];
    }
    if (objects.count) [lines addObject:[@"Possible image subjects: " stringByAppendingString:[objects componentsJoinedByString:@", "]]];
    return [lines componentsJoinedByString:@"\n"];
}

char *sumi_read_document(const unsigned char *bytes, size_t length, const char *kind) {
    @autoreleasepool {
        NSData *data=[NSData dataWithBytes:bytes length:length];
        NSString *ext=[NSString stringWithUTF8String:kind];
        NSString *value=nil;
        if ([ext isEqual:@"pdf"]) {
            PDFDocument *pdf=[[PDFDocument alloc] initWithData:data];
            NSMutableString *pages=[NSMutableString string];
            for (NSUInteger i=0;i<MIN(pdf.pageCount,100) && pages.length<65000;i++) {
                PDFPage *page=[pdf pageAtIndex:i];
                NSString *text=page.string;
                if (!text.length && i<10) {
                    NSImage *thumb=[page thumbnailOfSize:NSMakeSize(1100,1500) forBox:kPDFDisplayBoxCropBox];
                    CGImageRef image=[thumb CGImageForProposedRect:NULL context:nil hints:nil];
                    if (image) text=readImage(image);
                }
                if (text.length) [pages appendFormat:@"Page %lu\n%@\n",(unsigned long)i+1,text];
            }
            value=pages;
        } else if ([ext isEqual:@"doc"] || [ext isEqual:@"rtf"]) {
            NSString *type=[ext isEqual:@"doc"]?NSDocFormatTextDocumentType:NSRTFTextDocumentType;
            value=[[NSAttributedString alloc] initWithData:data options:@{NSDocumentTypeDocumentAttribute:type} documentAttributes:nil error:nil].string;
        } else {
            CGImageSourceRef source=CGImageSourceCreateWithData((__bridge CFDataRef)data,NULL);
            if (source) {
                NSDictionary *options=@{(__bridge NSString*)kCGImageSourceCreateThumbnailFromImageAlways:@YES,(__bridge NSString*)kCGImageSourceThumbnailMaxPixelSize:@1800,(__bridge NSString*)kCGImageSourceCreateThumbnailWithTransform:@YES};
                CGImageRef image=CGImageSourceCreateThumbnailAtIndex(source,0,(__bridge CFDictionaryRef)options);
                if(image){value=readImage(image);CGImageRelease(image);}
                CFRelease(source);
            }
        }
        return value.length ? strdup(value.UTF8String) : NULL;
    }
}
int sumi_trash(const char *path) {
    @autoreleasepool {
        NSURL *url=[NSURL fileURLWithPath:[NSString stringWithUTF8String:path] isDirectory:YES];
        return [[NSFileManager defaultManager] trashItemAtURL:url resultingItemURL:nil error:nil] ? 1 : 0;
    }
}

char *sumi_image_preview(const unsigned char *bytes,size_t length) {
    @autoreleasepool {
        NSData *input=[NSData dataWithBytes:bytes length:length];
        CGImageSourceRef source=CGImageSourceCreateWithData((__bridge CFDataRef)input,NULL);
        if(!source) return NULL;
        NSDictionary *options=@{(__bridge NSString*)kCGImageSourceCreateThumbnailFromImageAlways:@YES,(__bridge NSString*)kCGImageSourceThumbnailMaxPixelSize:@900,(__bridge NSString*)kCGImageSourceCreateThumbnailWithTransform:@YES};
        CGImageRef image=CGImageSourceCreateThumbnailAtIndex(source,0,(__bridge CFDictionaryRef)options);
        CFRelease(source);
        if(!image)return NULL;
        NSBitmapImageRep *rep=[[NSBitmapImageRep alloc]initWithCGImage:image];
        CGImageRelease(image);
        NSData *data=[rep representationUsingType:NSBitmapImageFileTypeJPEG properties:@{NSImageCompressionFactor:@0.68}];
        if(!data.length || data.length>450000)return NULL;
        return strdup([[data base64EncodedStringWithOptions:0] UTF8String]);
    }
}

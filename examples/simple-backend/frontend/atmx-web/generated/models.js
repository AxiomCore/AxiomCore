// GENERATED CODE – DO NOT EDIT.
/* eslint-disable @typescript-eslint/no-explicit-any */
export const Mappers = {
    pyExample: {
        Item: {
            fromJson: (json) => ({
                description: json["description"],
                id: json["id"],
                ownerId: json["owner_id"],
                title: json["title"],
            }),
            toJson: (obj) => ({
                "description": obj.description,
                "id": obj.id,
                "owner_id": obj.ownerId,
                "title": obj.title,
            })
        },
        ItemCreate: {
            fromJson: (json) => ({
                description: json["description"],
                title: json["title"],
            }),
            toJson: (obj) => ({
                "description": obj.description,
                "title": obj.title,
            })
        },
        Token: {
            fromJson: (json) => ({
                accessToken: json["access_token"],
                tokenType: json["token_type"],
            }),
            toJson: (obj) => ({
                "access_token": obj.accessToken,
                "token_type": obj.tokenType,
            })
        },
        User: {
            fromJson: (json) => ({
                email: json["email"],
                id: json["id"],
                role: json["role"],
            }),
            toJson: (obj) => ({
                "email": obj.email,
                "id": obj.id,
                "role": obj.role,
            })
        },
        UserCreate: {
            fromJson: (json) => ({
                email: json["email"],
                password: json["password"],
            }),
            toJson: (obj) => ({
                "email": obj.email,
                "password": obj.password,
            })
        },
    },
};
